//! Optimized memory operations with alt-instruction dispatch.
//!
//! Provides dispatched kernel memory operations (`kernel_memcpy`,
//! `kernel_memzero`, `kernel_memmove`, `kernel_memcmp`) that are patched
//! at boot to the best available implementation via [`alt_fn!`].

use super::cpuid::CpuFeatures;

// ===========================================================================
// kernel_memcpy
// ===========================================================================

/// Copies `len` bytes from `src` to `dst` using `rep movsb`.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
/// - `src` must be valid for `len`-byte reads.
/// - Memory regions must not overlap.
unsafe fn memcpy_baseline(dst: *mut u8, src: *const u8, len: usize) {
    unsafe {
        core::arch::asm!(
            "rep movsb",
            inout("rdi") dst => _,
            inout("rsi") src => _,
            inout("rcx") len => _,
            options(nostack, preserves_flags),
        );
    }
}

#[cfg(hadron_kernel_fpu)]
unsafe fn memcpy_sse2(dst: *mut u8, src: *const u8, len: usize) {
    if len < hadron_intrinsics::x86_64::sse2::SSE2_MEMCPY_THRESHOLD {
        unsafe { memcpy_baseline(dst, src, len) };
        return;
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    unsafe { hadron_intrinsics::x86_64::sse2::memcpy_sse2_inner(dst, src, len) };
}

crate::alt_fn! {
    /// Dispatched kernel memcpy — patched at boot to the best implementation.
    pub fn kernel_memcpy(dst: *mut u8, src: *const u8, len: usize),
    baseline = memcpy_baseline,
    alternatives = [
        #[cfg(hadron_kernel_fpu)]
        (CpuFeatures::SSE2, 1, memcpy_sse2),
    ]
}

// ===========================================================================
// kernel_memzero
// ===========================================================================

/// Zeroes `len` bytes at `dst` using `rep stosb`.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
unsafe fn memzero_baseline(dst: *mut u8, len: usize) {
    unsafe {
        core::arch::asm!(
            "xor eax, eax",
            "rep stosb",
            inout("rdi") dst => _,
            inout("rcx") len => _,
            out("rax") _,
            options(nostack),
        );
    }
}

#[cfg(hadron_kernel_fpu)]
unsafe fn memzero_sse2(dst: *mut u8, len: usize) {
    if len < hadron_intrinsics::x86_64::sse2::SSE2_MEMZERO_THRESHOLD {
        unsafe { memzero_baseline(dst, len) };
        return;
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    unsafe { hadron_intrinsics::x86_64::sse2::memzero_sse2_inner(dst, len) };
}

/// ERMS memzero — same `rep stosb` instruction but wins on ERMS-capable CPUs
/// because the hardware-optimized microcode is faster than SSE2 stores.
unsafe fn memzero_erms(dst: *mut u8, len: usize) {
    unsafe { memzero_baseline(dst, len) };
}

crate::alt_fn! {
    /// Dispatched kernel memzero — patched at boot to the best implementation.
    pub fn kernel_memzero(dst: *mut u8, len: usize),
    baseline = memzero_baseline,
    alternatives = [
        #[cfg(hadron_kernel_fpu)]
        (CpuFeatures::SSE2, 1, memzero_sse2),
        (CpuFeatures::ERMS, 2, memzero_erms),
    ]
}

// ===========================================================================
// kernel_memmove
// ===========================================================================

/// Moves `len` bytes from `src` to `dst`, handling overlapping regions.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
/// - `src` must be valid for `len`-byte reads.
unsafe fn memmove_baseline(dst: *mut u8, src: *const u8, len: usize) {
    // SAFETY: core::ptr::copy handles overlapping regions correctly.
    unsafe { core::ptr::copy(src, dst, len) };
}

#[cfg(hadron_kernel_fpu)]
unsafe fn memmove_sse2(dst: *mut u8, src: *const u8, len: usize) {
    if len < hadron_intrinsics::x86_64::sse2::SSE2_MEMCPY_THRESHOLD {
        unsafe { memmove_baseline(dst, src, len) };
        return;
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    unsafe { hadron_intrinsics::x86_64::sse2::memmove_sse2_inner(dst, src, len) };
}

crate::alt_fn! {
    /// Dispatched kernel memmove — patched at boot to the best implementation.
    pub fn kernel_memmove(dst: *mut u8, src: *const u8, len: usize),
    baseline = memmove_baseline,
    alternatives = [
        #[cfg(hadron_kernel_fpu)]
        (CpuFeatures::SSE2, 1, memmove_sse2),
    ]
}

// ===========================================================================
// kernel_memcmp
// ===========================================================================

/// Compares `len` bytes at `a` and `b`, returning 0 if equal, negative if
/// `a < b`, positive if `a > b`.
///
/// # Safety
///
/// - `a` and `b` must be valid for `len`-byte reads.
unsafe fn memcmp_baseline(a: *const u8, b: *const u8, len: usize) -> i32 {
    for i in 0..len {
        let va = unsafe { *a.add(i) };
        let vb = unsafe { *b.add(i) };
        if va != vb {
            return (va as i32) - (vb as i32);
        }
    }
    0
}

#[cfg(hadron_kernel_fpu)]
unsafe fn memcmp_sse2(a: *const u8, b: *const u8, len: usize) -> i32 {
    if len < 32 {
        return unsafe { memcmp_baseline(a, b, len) };
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    unsafe { hadron_intrinsics::x86_64::sse2::memcmp_sse2_inner(a, b, len) }
}

crate::alt_fn! {
    /// Dispatched kernel memcmp — patched at boot to the best implementation.
    pub fn kernel_memcmp(a: *const u8, b: *const u8, len: usize) -> i32,
    baseline = memcmp_baseline,
    alternatives = [
        #[cfg(hadron_kernel_fpu)]
        (CpuFeatures::SSE2, 1, memcmp_sse2),
    ]
}
