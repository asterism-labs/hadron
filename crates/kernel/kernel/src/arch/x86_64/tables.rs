//! Instructions that load and read the GDT, the LDT, the task register and
//! the segment registers. The kernel's only copies of them live here.

use core::arch::asm;

use arch_x86_64::{DescriptorTablePointer, SegmentSelector};

/// Loads the GDT register.
///
/// # Safety
///
/// `pointer` must describe a valid GDT that stays at that address while any
/// CPU uses it, and whose code and data descriptors match the segment
/// registers until they are reloaded.
pub(crate) unsafe fn lgdt(pointer: &DescriptorTablePointer) {
    // SAFETY: the caller vouches for the table. `lgdt` reads only the
    // ten-byte operand.
    unsafe { asm!("lgdt [{}]", in(reg) pointer, options(readonly, nostack, preserves_flags)) };
}

/// Reads the GDT register.
pub(crate) fn sgdt() -> DescriptorTablePointer {
    let mut pointer = DescriptorTablePointer { limit: 0, base: 0 };
    // SAFETY: `sgdt` writes only the ten-byte operand, which is `pointer`.
    unsafe { asm!("sgdt [{}]", in(reg) &raw mut pointer, options(nostack, preserves_flags)) };
    pointer
}

/// Reloads CS with `selector` through a far return.
///
/// # Safety
///
/// `selector` must name a present 64-bit code segment at ring 0 in the
/// loaded GDT.
pub(crate) unsafe fn load_code_segment(selector: SegmentSelector) {
    // SAFETY: the caller vouches for the selector. The far return pops the
    // two values pushed here and lands on the next instruction.
    unsafe {
        asm!(
            "push {selector}",
            "lea {target}, [rip + 2f]",
            "push {target}",
            "retfq",
            "2:",
            selector = in(reg) u64::from(selector.bits()),
            target = lateout(reg) _,
            options(preserves_flags),
        );
    }
}

/// Loads DS, ES and SS with `selector`. FS and GS are left alone, since
/// their bases come from MSRs.
///
/// # Safety
///
/// `selector` must name a present, writable data segment at ring 0 in the
/// loaded GDT.
pub(crate) unsafe fn load_data_segments(selector: SegmentSelector) {
    // SAFETY: the caller vouches for the selector. In 64-bit mode these
    // segments have base 0 and no limit, so memory access does not change.
    unsafe {
        asm!(
            "mov ds, {0:x}",
            "mov es, {0:x}",
            "mov ss, {0:x}",
            in(reg) selector.bits(),
            options(nostack, preserves_flags),
        );
    }
}

/// Reads CS.
pub(crate) fn code_segment() -> SegmentSelector {
    let bits: u16;
    // SAFETY: reading CS has no side effects.
    unsafe { asm!("mov {0:x}, cs", out(reg) bits, options(nomem, nostack, preserves_flags)) };
    SegmentSelector::from_bits(bits)
}

/// Loads the task register. The CPU marks the TSS descriptor busy in the
/// GDT.
///
/// # Safety
///
/// `selector` must name an available 64-bit TSS descriptor in the loaded
/// GDT, whose TSS stays at its address while the CPU uses it. The GDT must
/// be writable, and no Rust reference to it may be in use across the call.
pub(crate) unsafe fn ltr(selector: SegmentSelector) {
    // SAFETY: the caller vouches for the descriptor. `ltr` writes only the
    // descriptor's type field.
    unsafe { asm!("ltr {0:x}", in(reg) selector.bits(), options(nostack, preserves_flags)) };
}

/// Reads the task register.
pub(crate) fn task_register() -> SegmentSelector {
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
pub(crate) unsafe fn lldt(selector: SegmentSelector) {
    // SAFETY: the caller vouches for the selector.
    unsafe {
        asm!("lldt {0:x}", in(reg) selector.bits(), options(readonly, nostack, preserves_flags));
    }
}

/// Loads the IDT register.
///
/// # Safety
///
/// `pointer` must describe a valid IDT that stays at that address while
/// any CPU uses it, and whose gates point at handlers that follow the
/// interrupt calling convention.
pub(crate) unsafe fn lidt(pointer: &DescriptorTablePointer) {
    // SAFETY: the caller vouches for the table. `lidt` reads only the
    // ten-byte operand.
    unsafe {
        asm!("lidt [{}]", in(reg) pointer, options(readonly, nostack, preserves_flags));
    }
}

/// Reads the IDT register.
pub(crate) fn sidt() -> DescriptorTablePointer {
    let mut pointer = DescriptorTablePointer { limit: 0, base: 0 };
    // SAFETY: `sidt` writes only the ten-byte operand, which is `pointer`.
    unsafe {
        asm!("sidt [{}]", in(reg) &raw mut pointer, options(nostack, preserves_flags));
    }
    pointer
}
