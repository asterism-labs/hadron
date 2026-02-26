//! Thread-local mocks for interrupt and CPU-local state under loom.
//!
//! Loom cannot model hardware interrupts or per-CPU storage. These mocks
//! provide thread-local stand-ins so that [`IrqSpinLock`](super::IrqSpinLock)
//! loom tests can verify:
//!
//! - Interrupts are disabled while the lock is held
//! - Flag save/restore is correct on drop
//! - Nested acquisition correctness
//!
//! Each loom thread gets its own independent interrupt state, simulating
//! the per-CPU nature of real interrupt flags.

use std::cell::Cell;

std::thread_local! {
    /// Simulated interrupt-enabled flag (per-thread, mimicking per-CPU).
    static IRQ_ENABLED: Cell<bool> = const { Cell::new(true) };
    /// Simulated CPU ID (per-thread).
    static CPU_ID: Cell<u32> = const { Cell::new(0) };
}

/// Save the current interrupt state and disable interrupts.
///
/// Returns a flags value encoding whether interrupts were enabled.
/// Mirrors the real `save_flags_and_cli()` in `irq_spinlock.rs`.
pub(crate) fn mock_save_flags_and_cli() -> u64 {
    IRQ_ENABLED.with(|cell| {
        let was_enabled = cell.get();
        cell.set(false);
        if was_enabled { 1 << 9 } else { 0 }
    })
}

/// Restore interrupt state from a previously saved flags value.
///
/// If the IF bit (bit 9) is set in `flags`, interrupts are re-enabled.
pub(crate) fn mock_restore_flags(flags: u64) {
    if flags & (1 << 9) != 0 {
        IRQ_ENABLED.with(|cell| cell.set(true));
    }
}

/// Query whether interrupts are currently enabled on this thread.
pub(crate) fn mock_irq_enabled() -> bool {
    IRQ_ENABLED.with(Cell::get)
}

/// Set the simulated CPU ID for the current thread.
#[allow(dead_code)] // Phase 2: used by future per-CPU loom tests
pub(crate) fn mock_set_cpu_id(id: u32) {
    CPU_ID.with(|cell| cell.set(id));
}

/// Get the simulated CPU ID for the current thread.
#[allow(dead_code)] // Phase 2: used by future per-CPU loom tests
pub(crate) fn mock_get_cpu_id() -> u32 {
    CPU_ID.with(Cell::get)
}
