//! Per-CPU ring buffer for profiling samples.
//!
//! A fixed-size ring buffer accessed only from ISR context on the owning
//! CPU. No locking required — single-producer, drained at stop time.

use core::cell::UnsafeCell;

/// A profiling sample captured from a timer interrupt.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Sample {
    /// TSC timestamp of the sample.
    pub tsc: u64,
    /// CPU ID that captured this sample.
    pub cpu_id: u32,
    /// Number of valid entries in `stack`.
    pub depth: u32,
    /// Return addresses (top of stack first, bottom last).
    pub stack: [u64; MAX_SAMPLE_DEPTH],
}

/// Maximum stack depth per sample (from Kconfig).
pub const MAX_SAMPLE_DEPTH: usize = hadron_config::PROFILE_SAMPLE_DEPTH as usize;

/// Number of entries in the per-CPU ring buffer (from Kconfig).
const BUFFER_ENTRIES: usize = hadron_config::PROFILE_SAMPLE_BUFFER as usize;

/// Per-CPU sample ring buffer.
///
/// Only accessed from the timer ISR on the owning CPU. Overwrites
/// oldest entries on overflow (ring buffer semantics).
pub struct SampleRingBuf {
    entries: [Sample; BUFFER_ENTRIES],
    /// Next write position (wraps around).
    write_idx: usize,
    /// Number of samples written (may exceed `BUFFER_ENTRIES`).
    total_written: usize,
}

impl SampleRingBuf {
    /// Create a zeroed ring buffer.
    pub const fn new() -> Self {
        Self {
            entries: [Sample {
                tsc: 0,
                cpu_id: 0,
                depth: 0,
                stack: [0; MAX_SAMPLE_DEPTH],
            }; BUFFER_ENTRIES],
            write_idx: 0,
            total_written: 0,
        }
    }

    /// Push a sample into the ring buffer, overwriting the oldest if full.
    ///
    /// # Safety
    ///
    /// Must only be called from the timer ISR on the owning CPU.
    pub fn push(&mut self, sample: Sample) {
        self.entries[self.write_idx] = sample;
        self.write_idx = (self.write_idx + 1) % BUFFER_ENTRIES;
        self.total_written += 1;
    }

    /// Drain all valid samples, calling `f` for each.
    ///
    /// After drain, the buffer is logically empty.
    pub fn drain(&mut self, mut f: impl FnMut(&Sample)) {
        let count = self.total_written.min(BUFFER_ENTRIES);
        if count == 0 {
            return;
        }

        // If we've wrapped, start from write_idx (oldest). Otherwise start from 0.
        let start = if self.total_written > BUFFER_ENTRIES {
            self.write_idx
        } else {
            0
        };

        for i in 0..count {
            let idx = (start + i) % BUFFER_ENTRIES;
            f(&self.entries[idx]);
        }

        self.write_idx = 0;
        self.total_written = 0;
    }

    /// Returns the number of samples currently stored.
    pub fn len(&self) -> usize {
        self.total_written.min(BUFFER_ENTRIES)
    }
}

/// Wrapper for per-CPU access in ISR context.
///
/// Only the owning CPU's timer ISR writes; drain happens with interrupts
/// disabled on the same CPU.
pub struct SampleBufCell(pub UnsafeCell<SampleRingBuf>);

// SAFETY: SampleBufCell is only accessed from the timer ISR on the owning
// CPU (single-producer) and drained with interrupts disabled. No concurrent
// access is possible.
unsafe impl Send for SampleBufCell {}
unsafe impl Sync for SampleBufCell {}

impl SampleBufCell {
    /// Create a new cell with a zeroed buffer.
    pub const fn new() -> Self {
        Self(UnsafeCell::new(SampleRingBuf::new()))
    }
}
