//! Miscellaneous CPU instructions.

/// Swaps the GS base register with the `IA32_KERNEL_GS_BASE` MSR.
///
/// # Safety
///
/// Misusing SWAPGS corrupts the GS base, breaking per-CPU data access.
/// Must only be called at ring transitions (syscall entry/exit, interrupt
/// entry/exit when crossing privilege levels).
#[inline]
pub unsafe fn swapgs() {
    // SAFETY: Caller guarantees this is called at a valid ring transition.
    unsafe {
        core::arch::asm!("swapgs", options(nomem, nostack, preserves_flags));
    }
}

/// Reads the Time Stamp Counter (TSC) via RDTSC.
///
/// Returns the 64-bit TSC value. Note: RDTSC is not serializing; use
/// [`rdtscp`] or an explicit serializing instruction (e.g., LFENCE)
/// if ordering guarantees are needed.
#[inline]
pub fn rdtsc() -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: RDTSC has no side effects.
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack),
        );
    }
    (hi as u64) << 32 | lo as u64
}

/// Reads the Time Stamp Counter via RDTSCP (serializing variant).
///
/// Returns `(tsc, aux)` where `aux` is the IA32_TSC_AUX value (typically
/// the processor ID set by the OS).
#[inline]
pub fn rdtscp() -> (u64, u32) {
    let (lo, hi, aux): (u32, u32, u32);
    // SAFETY: RDTSCP has no side effects beyond reading TSC and AUX.
    unsafe {
        core::arch::asm!(
            "rdtscp",
            out("eax") lo,
            out("edx") hi,
            out("ecx") aux,
            options(nomem, nostack),
        );
    }
    ((hi as u64) << 32 | lo as u64, aux)
}

/// Saves the FPU/SSE state to memory using FXSAVE64.
///
/// # Safety
///
/// - `ptr` must be 16-byte aligned and point to at least 512 bytes of
///   writable memory.
/// - Interrupts should typically be disabled to prevent concurrent FPU use.
#[inline]
pub unsafe fn fxsave64(ptr: *mut u8) {
    // SAFETY: Caller guarantees alignment and buffer size.
    unsafe {
        core::arch::asm!(
            "fxsave64 [{}]",
            in(reg) ptr,
            options(nostack),
        );
    }
}

/// Restores the FPU/SSE state from memory using FXRSTOR64.
///
/// # Safety
///
/// - `ptr` must be 16-byte aligned and point to a valid 512-byte FXSAVE area.
/// - Interrupts should typically be disabled to prevent concurrent FPU use.
#[inline]
pub unsafe fn fxrstor64(ptr: *const u8) {
    // SAFETY: Caller guarantees alignment and valid FXSAVE data.
    unsafe {
        core::arch::asm!(
            "fxrstor64 [{}]",
            in(reg) ptr,
            options(nostack),
        );
    }
}

/// Saves the extended processor state using XSAVE64.
///
/// `rfbm` is the Requested Feature Bitmap specifying which state components
/// to save.
///
/// # Safety
///
/// - `ptr` must be 64-byte aligned and point to a sufficiently large
///   writable buffer (size depends on enabled state components).
/// - CR4.OSXSAVE must be set.
/// - Interrupts should typically be disabled to prevent concurrent FPU use.
#[inline]
pub unsafe fn xsave64(ptr: *mut u8, rfbm: u64) {
    // SAFETY: Caller guarantees alignment, buffer size, and CR4.OSXSAVE.
    unsafe {
        core::arch::asm!(
            "xsave64 [{}]",
            in(reg) ptr,
            in("eax") rfbm as u32,
            in("edx") (rfbm >> 32) as u32,
            options(nostack),
        );
    }
}

/// Restores the extended processor state using XRSTOR64.
///
/// `rfbm` is the Requested Feature Bitmap specifying which state components
/// to restore.
///
/// # Safety
///
/// - `ptr` must be 64-byte aligned and point to a valid XSAVE area.
/// - CR4.OSXSAVE must be set.
/// - Interrupts should typically be disabled to prevent concurrent FPU use.
#[inline]
pub unsafe fn xrstor64(ptr: *const u8, rfbm: u64) {
    // SAFETY: Caller guarantees alignment, valid data, and CR4.OSXSAVE.
    unsafe {
        core::arch::asm!(
            "xrstor64 [{}]",
            in(reg) ptr,
            in("eax") rfbm as u32,
            in("edx") (rfbm >> 32) as u32,
            options(nostack),
        );
    }
}
