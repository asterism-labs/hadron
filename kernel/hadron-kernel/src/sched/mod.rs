//! Kernel task scheduler.
//!
//! Core scheduler logic (executor, timer, waker, primitives) lives in the
//! `hadron-sched` crate for host testability. This module re-exports them
//! and adds kernel-specific code (SMP/IPI, block_on, sleep primitives).

// Re-export everything from hadron-sched root.
pub use hadron_sched::{
    Executor, Priority, TaskMeta,
    clear_preempt_pending, preempt_pending, set_preempt_pending,
    spawn, spawn_background, spawn_critical, spawn_with,
};

// Re-export submodules that don't need kernel extension.
pub use hadron_sched::timer;
pub use hadron_sched::waker;

/// Returns a reference to the current CPU's executor.
///
/// Convenience wrapper for [`hadron_sched::executor::global`].
#[inline]
pub fn executor() -> &'static Executor {
    hadron_sched::executor::global()
}

// Kernel-extended modules.
pub mod block_on;
pub mod primitives;
pub mod smp;

// ── ArchHalt implementation ─────────────────────────────────────────

/// x86_64 implementation of [`hadron_sched::executor::ArchHalt`].
///
/// Enables interrupts and halts (`sti; hlt`), then disables interrupts
/// after waking from the halt.
#[cfg(target_arch = "x86_64")]
pub struct X86ArchHalt;

#[cfg(target_arch = "x86_64")]
impl hadron_sched::executor::ArchHalt for X86ArchHalt {
    fn enable_interrupts_and_halt(&self) {
        // SAFETY: IDT and LAPIC are fully configured before executor starts.
        unsafe {
            crate::arch::x86_64::instructions::interrupts::enable_and_hlt();
        }
        // Interrupt fired — disable interrupts and check for ready tasks.
        crate::arch::x86_64::instructions::interrupts::disable();
    }
}
