//! Bounded lock-free MPSC ring buffer for log records.
//!
//! Implements a Vyukov/Lamport bounded queue with 256 entries. Multiple
//! producers (any CPU, including ISRs) claim slots via CAS on the `write`
//! cursor, then publish by storing to the per-slot `sequence` counter.
//! A single consumer (`drain_all`) reads entries in order.
//!
//! Memory footprint: ~40 KB (vs ~655 KB for the old per-CPU `SmallRing` array).
//!
//! # ISR Safety
//!
//! If an ISR fires between a producer's successful CAS and the subsequent
//! `Release` store to `sequence`, the ISR simply claims the *next* slot.
//! The interrupted producer's slot remains unpublished until it resumes —
//! the consumer will not see it until the sequence number is written.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::record::LogRecord;

/// Number of entries in the ring buffer (must be a power of two).
const RING_SIZE: usize = 256;

/// Bitmask for wrapping indices into the ring.
const RING_MASK: usize = RING_SIZE - 1;

/// Bounded lock-free MPSC ring buffer for [`LogRecord`]s.
///
/// Producers CAS on `write` to claim a slot, write the record, then
/// `Release`-store to `sequence[idx]` to publish. The single consumer
/// checks `sequence[idx]` with `Acquire`, reads the record, then stores
/// `seq + RING_SIZE` to release the slot for reuse.
pub(crate) struct MpscRing {
    /// Storage for log records.
    entries: [UnsafeCell<MaybeUninit<LogRecord>>; RING_SIZE],
    /// Per-slot sequence counters. Initialized to `[0, 1, 2, ..., 255]`.
    /// A slot at index `i` is available for writing when `sequence[i] == write_ticket`.
    /// It is available for reading when `sequence[i] == write_ticket + 1`.
    sequence: [AtomicU32; RING_SIZE],
    /// Write cursor — producers CAS on this to claim a slot.
    write: AtomicU32,
    /// Read cursor — single consumer advances this.
    read: AtomicU32,
}

/// Initializes the sequence array with values `[0, 1, 2, ..., RING_SIZE - 1]`.
const fn init_sequence() -> [AtomicU32; RING_SIZE] {
    let mut arr = [const { AtomicU32::new(0) }; RING_SIZE];
    let mut i = 0;
    while i < RING_SIZE {
        arr[i] = AtomicU32::new(i as u32);
        i += 1;
    }
    arr
}

impl MpscRing {
    /// Creates a new ring buffer with all slots available.
    const fn new() -> Self {
        Self {
            entries: [const { UnsafeCell::new(MaybeUninit::uninit()) }; RING_SIZE],
            sequence: init_sequence(),
            write: AtomicU32::new(0),
            read: AtomicU32::new(0),
        }
    }

    /// Pushes a log record into the ring buffer.
    ///
    /// Returns `true` if the record was stored, `false` if the ring is full
    /// (the record is silently dropped).
    pub(crate) fn push(&self, record: LogRecord) -> bool {
        let mut pos = self.write.load(Ordering::Relaxed);
        loop {
            let idx = (pos as usize) & RING_MASK;
            let seq = self.sequence[idx].load(Ordering::Acquire);
            let diff = seq as i32 - pos as i32;

            if diff == 0 {
                // Slot is available — try to claim it.
                match self.write.compare_exchange_weak(
                    pos,
                    pos.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        // SAFETY: We own this slot exclusively. No other
                        // producer can claim it (CAS succeeded), and the
                        // consumer will not read it until we publish via
                        // the sequence store below.
                        unsafe {
                            (*self.entries[idx].get()).write(record);
                        }
                        // Publish: consumer sees this entry when it loads
                        // sequence[idx] and finds pos + 1.
                        self.sequence[idx].store(pos.wrapping_add(1), Ordering::Release);
                        return true;
                    }
                    Err(actual) => {
                        pos = actual;
                    }
                }
            } else if diff < 0 {
                // Slot is occupied and not yet consumed — ring is full.
                return false;
            } else {
                // Another producer claimed this slot; reload and retry.
                pos = self.write.load(Ordering::Relaxed);
            }
        }
    }

    /// Pops the next published record from the ring buffer.
    ///
    /// Called by the single consumer (drain task). Returns `None` if no
    /// entries are ready.
    pub(crate) fn pop(&self) -> Option<LogRecord> {
        let pos = self.read.load(Ordering::Relaxed);
        let idx = (pos as usize) & RING_MASK;
        let seq = self.sequence[idx].load(Ordering::Acquire);

        // The entry is ready when sequence[idx] == pos + 1 (published by producer).
        if seq as i32 - pos.wrapping_add(1) as i32 != 0 {
            return None;
        }

        // SAFETY: The producer has fully written this entry (guaranteed
        // by the Release store to sequence[idx] that we synchronized with
        // via our Acquire load). We are the sole consumer.
        let record = unsafe { (*self.entries[idx].get()).assume_init_read() };

        // Release the slot: set sequence to pos + RING_SIZE so producers
        // can reclaim it in a future wrap-around.
        self.sequence[idx].store(pos.wrapping_add(RING_SIZE as u32), Ordering::Release);
        self.read.store(pos.wrapping_add(1), Ordering::Relaxed);

        Some(record)
    }
}

// SAFETY: `MpscRing` is a lock-free MPSC queue. Multiple producers
// synchronize via CAS on `write` and per-slot `sequence` counters.
// The single consumer synchronizes via `Acquire` loads of `sequence`
// and owns the `read` cursor exclusively. The `UnsafeCell<MaybeUninit>`
// entries are accessed in a mutually exclusive manner: a producer writes
// only after claiming a slot (CAS), and the consumer reads only after
// the producer publishes (sequence store with Release). No two threads
// access the same slot simultaneously.
unsafe impl Sync for MpscRing {}

// SAFETY: MpscRing contains only atomics and UnsafeCell<MaybeUninit<LogRecord>>.
// LogRecord is Send, so transferring ownership across threads is safe.
unsafe impl Send for MpscRing {}

/// Global MPSC ring buffer for all log records.
pub(crate) static RING: MpscRing = MpscRing::new();

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
        let ring = MpscRing::new();
        assert!(ring.push(make_record(42)));
        let r = ring.pop().unwrap();
        assert_eq!(r.timestamp, 42);
        assert!(ring.pop().is_none());
    }

    #[test]
    fn push_pop_fill() {
        let ring = MpscRing::new();
        for i in 0..RING_SIZE as u64 {
            assert!(ring.push(make_record(i)));
        }
        for i in 0..RING_SIZE as u64 {
            let r = ring.pop().unwrap();
            assert_eq!(r.timestamp, i);
        }
        assert!(ring.pop().is_none());
    }

    #[test]
    fn overflow_drops() {
        let ring = MpscRing::new();
        // Fill the ring
        for i in 0..RING_SIZE as u64 {
            assert!(ring.push(make_record(i)));
        }
        // Next push should fail (ring full, no consumer draining)
        assert!(!ring.push(make_record(999)));
    }

    #[test]
    fn multi_producer() {
        use std::sync::Arc;
        use std::thread;

        let ring = Arc::new(MpscRing::new());
        let num_threads = 4;
        let per_thread = 50;
        let mut handles = Vec::new();

        for t in 0..num_threads {
            let ring = Arc::clone(&ring);
            handles.push(thread::spawn(move || {
                let mut pushed = 0u64;
                for i in 0..per_thread {
                    let tag = (t as u64) * 1000 + i;
                    if ring.push(make_record(tag)) {
                        pushed += 1;
                    }
                }
                pushed
            }));
        }

        let total_pushed: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();

        let mut total_popped = 0u64;
        while ring.pop().is_some() {
            total_popped += 1;
        }

        assert_eq!(total_pushed, total_popped);
        assert_eq!(total_pushed, (num_threads * per_thread) as u64);
    }

    #[test]
    fn sequence_wrap() {
        let ring = MpscRing::new();
        // Push and pop many times to wrap around u32 sequence space.
        // We can't truly wrap u32, but we can verify correctness over
        // many iterations (multiple ring wraps).
        let iterations = RING_SIZE * 10;
        for i in 0..iterations as u64 {
            assert!(ring.push(make_record(i)));
            let r = ring.pop().unwrap();
            assert_eq!(r.timestamp, i);
        }
        assert!(ring.pop().is_none());
    }
}
