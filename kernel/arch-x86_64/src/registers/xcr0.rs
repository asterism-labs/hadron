//! Extended Control Register 0 (XCR0).
//!
//! Controls which processor state components are enabled for XSAVE/XRSTOR.
//!
//! # References
//!
//! - Intel SDM Vol. 1, §13.3: XCR0 and the XSTATE_BV Field
//!   <https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html>

bitflags::bitflags! {
    /// XCR0 register flags controlling XSAVE-managed state components.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Xcr0Flags: u64 {
        /// x87 FPU state (always set).
        const X87 = 1 << 0;
        /// SSE state (XMM registers).
        const SSE = 1 << 1;
        /// AVX state (upper halves of YMM registers).
        const AVX = 1 << 2;
    }
}

/// Reads XCR0 (Extended Control Register 0) via XGETBV with ECX=0.
///
/// # Safety
///
/// The caller must ensure CR4.OSXSAVE is set before calling this function.
#[inline]
pub unsafe fn xgetbv() -> Xcr0Flags {
    let (lo, hi): (u32, u32);
    // SAFETY: Caller guarantees CR4.OSXSAVE is set.
    unsafe {
        core::arch::asm!(
            "xgetbv",
            in("ecx") 0u32,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }
    Xcr0Flags::from_bits_truncate((hi as u64) << 32 | lo as u64)
}

/// Writes XCR0 via XSETBV with ECX=0.
///
/// # Safety
///
/// The caller must ensure CR4.OSXSAVE is set and the value is valid for
/// the CPU's supported state components.
#[inline]
pub unsafe fn xsetbv(flags: Xcr0Flags) {
    let val = flags.bits();
    let lo = val as u32;
    let hi = (val >> 32) as u32;
    // SAFETY: Caller guarantees CR4.OSXSAVE is set and value is valid.
    unsafe {
        core::arch::asm!(
            "xsetbv",
            in("ecx") 0u32,
            in("eax") lo,
            in("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }
}
