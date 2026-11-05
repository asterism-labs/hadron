//! Port I/O. The kernel's only `in` and `out` instructions live here.

use core::arch::asm;

/// Reads one byte from `port`.
///
/// # Safety
///
/// Reading a device register can change the device's state. The caller
/// must own `port` and know what the read does.
pub(crate) unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: the caller owns `port`. `in` touches no memory.
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// Writes one byte to `port`.
///
/// # Safety
///
/// The caller must own `port` and know what the write does to the device.
pub(crate) unsafe fn outb(port: u16, value: u8) {
    // SAFETY: the caller owns `port`. `out` touches no memory.
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

/// Writes four bytes to `port`.
///
/// # Safety
///
/// The caller must own `port` and know what the write does to the device.
pub(crate) unsafe fn outl(port: u16, value: u32) {
    // SAFETY: the caller owns `port`. `out` touches no memory.
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack, preserves_flags));
    }
}
