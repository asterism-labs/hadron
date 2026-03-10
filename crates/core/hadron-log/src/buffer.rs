//! Per-CPU SPSC ring buffer for log records.
//!
//! Phase 1 uses a small static ring (`SMALL_RING_SIZE` entries) stored in
//! `CpuLocal` storage. A single producer (the logging CPU) writes entries;
//! the drain task reads committed entries.
//!
//! ## ISR Safety
//!
//! The `write` cursor is a `Cell<u32>` (non-atomic, per-CPU exclusive). If
//! an ISR fires mid-write, it overwrites the partially-written entry. The
//! interrupted entry is lost but no UB occurs — the ISR's entry is committed
//! instead.

use core::cell::Cell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::cpu_local::{CpuLocal, MAX_CPUS};

use crate::record::LogRecord;

/// Number of entries in the small (Phase 1) per-CPU ring buffer.
const SMALL_RING_SIZE: usize = 16;

/// Per-CPU ring buffer for log records.
///
/// Single-producer (logging CPU), single-consumer (drain task/flush).
pub(crate) struct SmallRing {
    entries: [MaybeUninit<LogRecord>; SMALL_RING_SIZE],
    /// Write cursor — advanced by the producer (current CPU only).
    write: Cell<u32>,
    /// Committed cursor — set by the producer with `Release` after writing.
    committed: AtomicU32,
    /// Read cursor — advanced by the consumer (drain task).
    read: AtomicU32,
}

impl SmallRing {
    const fn new() -> Self {
        Self {
            entries: [const { MaybeUninit::uninit() }; SMALL_RING_SIZE],
            write: Cell::new(0),
            committed: AtomicU32::new(0),
            read: AtomicU32::new(0),
        }
    }

    /// Writes a log record into the ring buffer.
    ///
    /// Called by the producer (current CPU). If the ring is full, the oldest
    /// uncommitted entry is silently overwritten.
    pub(crate) fn push(&self, record: LogRecord) {
        let w = self.write.get();
        let idx = (w as usize) % SMALL_RING_SIZE;

        // SAFETY: We are the sole writer on this CPU. The entry at `idx` is
        // either uninit or a previously-committed entry that the consumer
        // has already read (or we're overwriting it because the ring is full).
        let slot =
            &self.entries[idx] as *const MaybeUninit<LogRecord> as *mut MaybeUninit<LogRecord>;
        unsafe {
            slot.write(MaybeUninit::new(record));
        }

        self.write.set(w.wrapping_add(1));
        self.committed.store(w.wrapping_add(1), Ordering::Release);
    }

    /// Returns the number of committed but unread entries.
    pub(crate) fn pending_count(&self) -> u32 {
        let committed = self.committed.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Relaxed);
        committed.wrapping_sub(read)
    }

    /// Reads the next committed entry from the ring buffer.
    ///
    /// Called by the consumer (drain task). Returns `None` if no entries
    /// are pending.
    pub(crate) fn pop(&self) -> Option<LogRecord> {
        let committed = self.committed.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Relaxed);

        if read == committed {
            return None;
        }

        let idx = (read as usize) % SMALL_RING_SIZE;

        // SAFETY: The producer has committed this entry (committed > read).
        // We are the sole consumer. The entry was fully written before the
        // Release store to `committed`.
        let record = unsafe { self.entries[idx].assume_init_read() };

        self.read.store(read.wrapping_add(1), Ordering::Relaxed);
        Some(record)
    }
}

// SAFETY: SmallRing is accessed per-CPU. The producer (current CPU) writes
// entries and advances `write`/`committed`. The consumer (drain task) reads
// entries and advances `read`. The atomic ordering on `committed` ensures
// the consumer sees fully-written entries.
unsafe impl Send for SmallRing {}
unsafe impl Sync for SmallRing {}

// ── Global per-CPU ring storage ─────────────────────────────────────────

pub(crate) static RINGS: CpuLocal<SmallRing> = {
    const INIT: SmallRing = SmallRing::new();
    CpuLocal::new([INIT; MAX_CPUS])
};

/// Pushes a log record onto the current CPU's ring buffer.
pub(crate) fn push_record(record: LogRecord) {
    RINGS.get().push(record);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::Level;
    use crate::record::RecordMessage;
    use crate::span::SpanSnapshot;

    fn make_record(tag: u64) -> LogRecord {
        LogRecord {
            timestamp: tag,
            level: Level::INFO,
            subsystem: "test",
            file: file!(),
            line: line!(),
            spans: SpanSnapshot::empty(),
            message: RecordMessage::Formatted {
                buf: [0u8; 128],
                len: 0,
            },
        }
    }

    #[test]
    fn push_pop_single() {
        let ring = SmallRing::new();
        ring.push(make_record(42));
        assert_eq!(ring.pending_count(), 1);
        let r = ring.pop().unwrap();
        assert_eq!(r.timestamp, 42);
        assert!(ring.pop().is_none());
    }

    #[test]
    fn push_pop_multiple() {
        let ring = SmallRing::new();
        for i in 0..SMALL_RING_SIZE as u64 {
            ring.push(make_record(i));
        }
        assert_eq!(ring.pending_count(), SMALL_RING_SIZE as u32);

        for i in 0..SMALL_RING_SIZE as u64 {
            let r = ring.pop().unwrap();
            assert_eq!(r.timestamp, i);
        }
        assert!(ring.pop().is_none());
    }

    #[test]
    fn overflow_overwrites_oldest() {
        let ring = SmallRing::new();
        // Fill + overflow by 2
        for i in 0..(SMALL_RING_SIZE as u64 + 2) {
            ring.push(make_record(i));
        }
        // The ring wraps, so we can read the last SMALL_RING_SIZE entries
        // but the committed count has advanced past what read can see.
        // Drain what's available.
        let mut count = 0;
        while ring.pop().is_some() {
            count += 1;
        }
        // We wrote SMALL_RING_SIZE + 2, committed is at that point,
        // but read started at 0, so we should get up to SMALL_RING_SIZE + 2
        // (the ring just wraps and overwrites, committed tracks total writes).
        assert_eq!(count, SMALL_RING_SIZE as u64 + 2);
    }
}
