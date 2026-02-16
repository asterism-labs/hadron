//! Interrupt-related instructions.

use crate::arch::x86_64::registers::rflags::{self, RFlags};

/// Disables interrupts (CLI).
#[inline]
pub fn disable() {
    // SAFETY: CLI has no side effects beyond masking maskable interrupts.
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }
}

/// Enables interrupts (STI).
///
/// # Safety
///
/// The caller must ensure that enabling interrupts is safe in the current
/// context (e.g., IDT is properly configured).
#[inline]
pub unsafe fn enable() {
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
    }
}

/// Returns `true` if interrupts are currently enabled (IF flag set in RFLAGS).
#[inline]
pub fn are_enabled() -> bool {
    rflags::read().contains(RFlags::INTERRUPT_FLAG)
}

/// Halts the CPU until the next interrupt (HLT).
#[inline]
pub fn hlt() {
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}

/// Atomically enables interrupts and halts the CPU.
///
/// Uses `sti; hlt` to avoid the race where an interrupt arrives between
/// enabling interrupts and halting. The `sti` instruction delays interrupt
/// delivery until after the following instruction, so the CPU is guaranteed
/// to halt before servicing any pending interrupt.
///
/// # Safety
///
/// The caller must ensure that enabling interrupts is safe in the current
/// context (e.g., IDT is properly configured).
#[inline]
pub unsafe fn enable_and_hlt() {
    unsafe {
        core::arch::asm!("sti; hlt", options(nomem, nostack, preserves_flags));
    }
}

/// Triggers a breakpoint exception (INT3).
#[inline]
pub fn int3() {
    unsafe {
        core::arch::asm!("int3", options(nomem, nostack));
    }
}

/// Executes the given closure with interrupts disabled, restoring the
/// previous interrupt state afterward.
#[inline]
pub fn without_interrupts<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let was_enabled = are_enabled();
    if was_enabled {
        disable();
    }
    let result = f();
    if was_enabled {
        unsafe { enable() };
    }
    result
}
