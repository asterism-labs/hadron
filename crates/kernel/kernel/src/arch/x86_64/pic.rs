//! The legacy 8259 PIC pair, which the kernel keeps masked.
//!
//! Firmware may leave the PICs delivering interrupts at vectors that
//! overlap the CPU exceptions. The kernel will use the local APIC instead,
//! so it masks every PIC line before it enables interrupts.

use super::port;

/// Data port of the primary PIC. Writing it sets the interrupt mask.
const PRIMARY_DATA: u16 = 0x21;
/// Data port of the secondary PIC.
const SECONDARY_DATA: u16 = 0xa1;
/// A mask with every line masked.
const ALL_LINES: u8 = 0xff;

/// Masks every line on both PICs.
pub(crate) fn mask_all() {
    // SAFETY: ports 0x21 and 0xa1 belong to the PICs on PCs, and only this
    // module touches them. Writing the data port outside initialization
    // sets the mask register and nothing else.
    unsafe {
        port::outb(PRIMARY_DATA, ALL_LINES);
        port::outb(SECONDARY_DATA, ALL_LINES);
    }
}
