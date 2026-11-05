//! CPU control instructions.

use core::arch::asm;

/// Masks maskable interrupts on this CPU.
pub(crate) fn disable_interrupts() {
    // SAFETY: `cli` only clears the interrupt flag. Code that later relies on
    // interrupts would hang, so only the panic path and boot call this.
    unsafe { asm!("cli", options(nomem, nostack)) };
}

/// Stops this CPU for good. Interrupts are masked first, so `hlt` does not
/// return except for an NMI, which the loop absorbs.
pub(crate) fn halt_forever() -> ! {
    disable_interrupts();
    loop {
        // SAFETY: `hlt` stops the CPU until the next interrupt.
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}
