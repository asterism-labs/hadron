//! Table-related instructions (`lgdt`, `lidt`, `ltr`).

use crate::structures::{DescriptorTablePointer, SegmentSelector};

/// Loads a GDT from the given pointer.
///
/// # Safety
///
/// The pointer must reference a valid GDT, and the caller must ensure
/// segment registers are reloaded appropriately afterward.
#[inline]
pub unsafe fn lgdt(ptr: &DescriptorTablePointer) {
    // SAFETY: Caller guarantees the pointer references a valid GDT.
    unsafe {
        core::arch::asm!(
            "lgdt [{}]",
            in(reg) ptr,
            options(nostack, preserves_flags),
        );
    }
}

/// Loads an IDT from the given pointer.
///
/// # Safety
///
/// The pointer must reference a valid IDT.
#[inline]
pub unsafe fn lidt(ptr: &DescriptorTablePointer) {
    // SAFETY: Caller guarantees the pointer references a valid IDT.
    unsafe {
        core::arch::asm!(
            "lidt [{}]",
            in(reg) ptr,
            options(nostack, preserves_flags),
        );
    }
}

/// Loads the Task Register with the given selector.
///
/// # Safety
///
/// The selector must reference a valid TSS descriptor in the current GDT.
#[inline]
pub unsafe fn ltr(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector references a valid TSS descriptor.
    unsafe {
        core::arch::asm!(
            "ltr {:x}",
            in(reg) selector.as_u16(),
            options(nostack, preserves_flags),
        );
    }
}
