//! Loads and stores of the GDT, IDT, LDT and task registers.

use core::arch::asm;

use crate::structures::segment::SegmentSelector;
use crate::structures::table::DescriptorTablePointer;

/// Loads the GDT register.
///
/// # Safety
///
/// `pointer` must describe a valid GDT that stays at that address while any
/// CPU uses it, and whose code and data descriptors match the segment
/// registers until they are reloaded.
pub unsafe fn lgdt(pointer: &DescriptorTablePointer) {
    // SAFETY: the caller vouches for the table. `lgdt` reads only the
    // ten-byte operand.
    unsafe { asm!("lgdt [{}]", in(reg) pointer, options(readonly, nostack, preserves_flags)) };
}

/// Reads the GDT register.
#[must_use]
pub fn sgdt() -> DescriptorTablePointer {
    let mut pointer = DescriptorTablePointer { limit: 0, base: 0 };
    // SAFETY: `sgdt` writes only the ten-byte operand, which is `pointer`.
    unsafe { asm!("sgdt [{}]", in(reg) &raw mut pointer, options(nostack, preserves_flags)) };
    pointer
}

/// Loads the IDT register.
///
/// # Safety
///
/// `pointer` must describe a valid IDT that stays at that address while
/// any CPU uses it, and whose gates point at handlers that follow the
/// interrupt calling convention.
pub unsafe fn lidt(pointer: &DescriptorTablePointer) {
    // SAFETY: the caller vouches for the table. `lidt` reads only the
    // ten-byte operand.
    unsafe { asm!("lidt [{}]", in(reg) pointer, options(readonly, nostack, preserves_flags)) };
}

/// Reads the IDT register.
#[must_use]
pub fn sidt() -> DescriptorTablePointer {
    let mut pointer = DescriptorTablePointer { limit: 0, base: 0 };
    // SAFETY: `sidt` writes only the ten-byte operand, which is `pointer`.
    unsafe { asm!("sidt [{}]", in(reg) &raw mut pointer, options(nostack, preserves_flags)) };
    pointer
}

/// Loads the task register. The CPU marks the TSS descriptor busy in the
/// GDT.
///
/// # Safety
///
/// `selector` must name an available 64-bit TSS descriptor in the loaded
/// GDT, whose TSS stays at its address while the CPU uses it. The GDT must
/// be writable, and no Rust reference to it may be in use across the call.
pub unsafe fn ltr(selector: SegmentSelector) {
    // SAFETY: the caller vouches for the descriptor. `ltr` writes only the
    // descriptor's type field.
    unsafe { asm!("ltr {0:x}", in(reg) selector.bits(), options(nostack, preserves_flags)) };
}

/// Reads the task register.
#[must_use]
pub fn str() -> SegmentSelector {
    let bits: u16;
    // SAFETY: `str` has no side effects.
    unsafe { asm!("str {0:x}", out(reg) bits, options(nomem, nostack, preserves_flags)) };
    SegmentSelector::from_bits(bits)
}

/// Loads the LDT register. The null selector leaves the CPU with no LDT.
///
/// # Safety
///
/// `selector` must be null or name an LDT descriptor in the loaded GDT
/// whose table stays at its address while the CPU uses it.
pub unsafe fn lldt(selector: SegmentSelector) {
    // SAFETY: the caller vouches for the selector.
    unsafe {
        asm!("lldt {0:x}", in(reg) selector.bits(), options(readonly, nostack, preserves_flags));
    }
}

/// Reads the LDT register.
#[must_use]
pub fn sldt() -> SegmentSelector {
    let bits: u16;
    // SAFETY: `sldt` has no side effects.
    unsafe { asm!("sldt {0:x}", out(reg) bits, options(nomem, nostack, preserves_flags)) };
    SegmentSelector::from_bits(bits)
}
