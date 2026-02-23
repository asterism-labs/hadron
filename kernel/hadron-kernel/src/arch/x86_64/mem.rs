//! Registers optimized memory operation alternatives.
//!
//! Baseline implementations and dispatch points live in
//! [`hadron_core::mem`]. This module registers SSE2 and ERMS
//! alternatives via [`hadron_core::alt_fn_alternative!`].
//!
//! FPU/SSE is enabled via [`fpu::enable_fpu_raw()`] (a naked assembly
//! function) as the very first thing in AP boot — before any Rust code
//! that could trigger compiler-generated memcpy/memset. This eliminates
//! the chicken-and-egg problem and lets SSE2 wrappers omit runtime
//! CR4.OSFXSR checks.

use hadron_core::cpu_features::CpuFeatures;

// ===========================================================================
// SSE2 memcpy alternative
// ===========================================================================

#[cfg(hadron_kernel_fpu)]
unsafe fn memcpy_sse2(dst: *mut u8, src: *const u8, len: usize) {
    if len < hadron_intrinsics::x86_64::sse2::SSE2_MEMCPY_THRESHOLD {
        // SAFETY: Caller guarantees valid, non-overlapping regions.
        // Call baseline assembly directly to avoid recursion through builtins.
        unsafe { hadron_core::mem::memcpy_baseline(dst, src, len) };
        return;
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    // SAFETY: FPU guard active, caller guarantees valid regions.
    unsafe { hadron_intrinsics::x86_64::sse2::memcpy_sse2_inner(dst, src, len) };
}

hadron_core::alt_fn_alternative! {
    dispatch = hadron_core::mem::dispatch::kernel_memcpy,
    #[cfg(hadron_kernel_fpu)]
    (CpuFeatures::SSE2, 1, memcpy_sse2)
}

// ===========================================================================
// SSE2 + ERMS memzero alternatives
// ===========================================================================

#[cfg(hadron_kernel_fpu)]
unsafe fn memzero_sse2(dst: *mut u8, len: usize) {
    if len < hadron_intrinsics::x86_64::sse2::SSE2_MEMZERO_THRESHOLD {
        // SAFETY: Caller guarantees valid region.
        unsafe { hadron_core::mem::memzero_baseline(dst, len) };
        return;
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    // SAFETY: FPU guard active, caller guarantees valid region.
    unsafe { hadron_intrinsics::x86_64::sse2::memzero_sse2_inner(dst, len) };
}

/// ERMS memzero — same `rep stosb` instruction but wins on ERMS-capable CPUs
/// because the hardware-optimized microcode is faster than SSE2 stores.
unsafe fn memzero_erms(dst: *mut u8, len: usize) {
    // SAFETY: Caller guarantees valid region.
    unsafe { hadron_core::mem::memzero_baseline(dst, len) };
}

hadron_core::alt_fn_alternative! {
    dispatch = hadron_core::mem::dispatch::kernel_memzero,
    #[cfg(hadron_kernel_fpu)]
    (CpuFeatures::SSE2, 1, memzero_sse2)
}

hadron_core::alt_fn_alternative! {
    dispatch = hadron_core::mem::dispatch::kernel_memzero,
    (CpuFeatures::ERMS, 2, memzero_erms)
}

// ===========================================================================
// SSE2 memmove alternative
// ===========================================================================

#[cfg(hadron_kernel_fpu)]
unsafe fn memmove_sse2(dst: *mut u8, src: *const u8, len: usize) {
    if len < hadron_intrinsics::x86_64::sse2::SSE2_MEMCPY_THRESHOLD {
        // SAFETY: Caller guarantees valid regions.
        unsafe { hadron_core::mem::memmove_baseline(dst, src, len) };
        return;
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    // SAFETY: FPU guard active, caller guarantees valid regions.
    unsafe { hadron_intrinsics::x86_64::sse2::memmove_sse2_inner(dst, src, len) };
}

hadron_core::alt_fn_alternative! {
    dispatch = hadron_core::mem::dispatch::kernel_memmove,
    #[cfg(hadron_kernel_fpu)]
    (CpuFeatures::SSE2, 1, memmove_sse2)
}

// ===========================================================================
// SSE2 memcmp alternative
// ===========================================================================

/// Minimum length threshold for SSE2 memcmp.
const SSE2_MEMCMP_THRESHOLD: usize = 32;

#[cfg(hadron_kernel_fpu)]
unsafe fn memcmp_sse2(a: *const u8, b: *const u8, len: usize) -> i32 {
    if len < SSE2_MEMCMP_THRESHOLD {
        // SAFETY: Caller guarantees valid regions.
        return unsafe { hadron_core::mem::memcmp_baseline(a, b, len) };
    }
    let _fpu = super::fpu::KernelFpuGuard::new();
    // SAFETY: FPU guard active, caller guarantees valid regions.
    unsafe { hadron_intrinsics::x86_64::sse2::memcmp_sse2_inner(a, b, len) }
}

hadron_core::alt_fn_alternative! {
    dispatch = hadron_core::mem::dispatch::kernel_memcmp,
    #[cfg(hadron_kernel_fpu)]
    (CpuFeatures::SSE2, 1, memcmp_sse2)
}
