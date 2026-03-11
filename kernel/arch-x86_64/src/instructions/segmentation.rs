//! Segment register manipulation instructions.

use crate::structures::SegmentSelector;

/// Reloads the code segment register (CS) using a far return.
///
/// # Safety
///
/// The selector must reference a valid code segment descriptor.
#[inline]
pub unsafe fn set_cs(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector references a valid code segment.
    unsafe {
        core::arch::asm!(
            "push {sel:r}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            sel = in(reg) u64::from(selector.as_u16()),
            tmp = lateout(reg) _,
            options(preserves_flags),
        );
    }
}

/// Loads the data segment register (DS).
///
/// # Safety
///
/// The selector must reference a valid data segment descriptor (or be zero).
#[inline]
pub unsafe fn load_ds(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector is valid.
    unsafe {
        core::arch::asm!(
            "mov ds, {:x}",
            in(reg) selector.as_u16(),
            options(nostack, preserves_flags),
        );
    }
}

/// Loads the stack segment register (SS).
///
/// # Safety
///
/// The selector must reference a valid stack segment descriptor.
#[inline]
pub unsafe fn load_ss(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector is valid.
    unsafe {
        core::arch::asm!(
            "mov ss, {:x}",
            in(reg) selector.as_u16(),
            options(nostack, preserves_flags),
        );
    }
}

/// Loads the extra segment register (ES).
///
/// # Safety
///
/// The selector must reference a valid data segment descriptor (or be zero).
#[inline]
pub unsafe fn load_es(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector is valid.
    unsafe {
        core::arch::asm!(
            "mov es, {:x}",
            in(reg) selector.as_u16(),
            options(nostack, preserves_flags),
        );
    }
}

/// Loads the FS segment register.
///
/// # Safety
///
/// The selector must reference a valid data segment descriptor (or be zero).
#[inline]
pub unsafe fn load_fs(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector is valid.
    unsafe {
        core::arch::asm!(
            "mov fs, {:x}",
            in(reg) selector.as_u16(),
            options(nostack, preserves_flags),
        );
    }
}

/// Loads the GS segment register.
///
/// # Safety
///
/// The selector must reference a valid data segment descriptor (or be zero).
#[inline]
pub unsafe fn load_gs(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector is valid.
    unsafe {
        core::arch::asm!(
            "mov gs, {:x}",
            in(reg) selector.as_u16(),
            options(nostack, preserves_flags),
        );
    }
}

/// Loads the Task Register with the given TSS selector.
///
/// # Safety
///
/// The selector must reference a valid TSS descriptor in the current GDT.
#[inline]
pub unsafe fn load_tss(selector: SegmentSelector) {
    // SAFETY: Caller guarantees the selector references a valid TSS descriptor.
    unsafe {
        core::arch::asm!(
            "ltr {:x}",
            in(reg) selector.as_u16(),
            options(nostack, preserves_flags),
        );
    }
}

// ---------------------------------------------------------------------------
// Read functions (safe — reading segment registers has no side effects)
// ---------------------------------------------------------------------------

/// Reads the code segment register (CS).
#[inline]
pub fn read_cs() -> SegmentSelector {
    let val: u16;
    // SAFETY: Reading CS has no side effects.
    unsafe {
        core::arch::asm!("mov {:x}, cs", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    SegmentSelector::from_raw(val)
}

/// Reads the data segment register (DS).
#[inline]
pub fn read_ds() -> SegmentSelector {
    let val: u16;
    // SAFETY: Reading DS has no side effects.
    unsafe {
        core::arch::asm!("mov {:x}, ds", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    SegmentSelector::from_raw(val)
}

/// Reads the extra segment register (ES).
#[inline]
pub fn read_es() -> SegmentSelector {
    let val: u16;
    // SAFETY: Reading ES has no side effects.
    unsafe {
        core::arch::asm!("mov {:x}, es", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    SegmentSelector::from_raw(val)
}

/// Reads the FS segment register.
#[inline]
pub fn read_fs() -> SegmentSelector {
    let val: u16;
    // SAFETY: Reading FS has no side effects.
    unsafe {
        core::arch::asm!("mov {:x}, fs", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    SegmentSelector::from_raw(val)
}

/// Reads the GS segment register.
#[inline]
pub fn read_gs() -> SegmentSelector {
    let val: u16;
    // SAFETY: Reading GS has no side effects.
    unsafe {
        core::arch::asm!("mov {:x}, gs", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    SegmentSelector::from_raw(val)
}

/// Reads the stack segment register (SS).
#[inline]
pub fn read_ss() -> SegmentSelector {
    let val: u16;
    // SAFETY: Reading SS has no side effects.
    unsafe {
        core::arch::asm!("mov {:x}, ss", out(reg) val, options(nomem, nostack, preserves_flags));
    }
    SegmentSelector::from_raw(val)
}
