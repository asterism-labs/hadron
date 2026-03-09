//! Compiler builtin memory operations with alt-fn dispatch.
//!
//! Provides baseline `rep movsb`/`rep stosb` implementations and
//! [`alt_fn!`](crate::alt_fn) dispatch points for `memcpy`, `memset`,
//! `memmove`, `memcmp`, and `bcmp`. On the kernel target these are
//! exposed as `#[unsafe(no_mangle)] extern "C"` symbols, replacing the
//! `compiler_builtins` `mem` feature so every compiler-generated
//! copy/zero/move automatically uses the best available implementation
//! after boot-time patching.

// ===========================================================================
// Baselines (always available on the kernel target)
// ===========================================================================

/// Copies `len` bytes from `src` to `dst` using `rep movsb`.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
/// - `src` must be valid for `len`-byte reads.
/// - Memory regions must not overlap.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub unsafe fn memcpy_baseline(dst: *mut u8, src: *const u8, len: usize) {
    // SAFETY: Caller guarantees valid, non-overlapping regions.
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

/// Zeroes `len` bytes at `dst` using `rep stosb`.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub unsafe fn memzero_baseline(dst: *mut u8, len: usize) {
    // SAFETY: Caller guarantees valid region.
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

/// Moves `len` bytes from `src` to `dst`, handling overlapping regions.
///
/// Uses forward `rep movsb` when `dst <= src`, and backward copy via
/// `std; rep movsb; cld` when `dst > src` to handle overlap correctly.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
/// - `src` must be valid for `len`-byte reads.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub unsafe fn memmove_baseline(dst: *mut u8, src: *const u8, len: usize) {
    if len == 0 {
        return;
    }
    if dst as usize <= src as usize || dst as usize >= src as usize + len {
        // Non-overlapping or forward-safe: use forward rep movsb.
        // SAFETY: Caller guarantees valid regions; direction is safe.
        unsafe {
            core::arch::asm!(
                "rep movsb",
                inout("rdi") dst => _,
                inout("rsi") src => _,
                inout("rcx") len => _,
                options(nostack, preserves_flags),
            );
        }
    } else {
        // Overlapping backward: copy from end to start.
        // SAFETY: Caller guarantees valid regions. We set DF for backward
        // copy and clear it afterwards (x86_64 ABI requires DF=0).
        unsafe {
            core::arch::asm!(
                "std",
                "rep movsb",
                "cld",
                inout("rdi") dst.add(len - 1) => _,
                inout("rsi") src.add(len - 1) => _,
                inout("rcx") len => _,
                options(nostack),
            );
        }
    }
}

/// Compares `len` bytes at `a` and `b`, returning 0 if equal, negative if
/// `a < b`, positive if `a > b`.
///
/// # Safety
///
/// - `a` and `b` must be valid for `len`-byte reads.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub unsafe fn memcmp_baseline(a: *const u8, b: *const u8, len: usize) -> i32 {
    for i in 0..len {
        // SAFETY: Caller guarantees valid regions of at least `len` bytes.
        let va = unsafe { *a.add(i) };
        let vb = unsafe { *b.add(i) };
        if va != vb {
            return (va as i32) - (vb as i32);
        }
    }
    0
}

// ===========================================================================
// Alt-fn dispatch points (kernel target only)
// ===========================================================================

/// Dispatch points for the kernel memory operations.
///
/// Each dispatch point starts with the baseline implementation and is
/// patched at boot by [`hadron_kernel::arch::x86_64::alt_fn::apply()`]
/// to the best available alternative (SSE2, ERMS, etc.).
///
/// External crates register alternatives via
/// [`alt_fn_alternative!`](crate::alt_fn_alternative).
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
#[doc(hidden)]
pub mod dispatch {
    use super::{memcmp_baseline, memcpy_baseline, memmove_baseline, memzero_baseline};

    crate::alt_fn! {
        /// Dispatched kernel memcpy.
        pub fn kernel_memcpy(dst: *mut u8, src: *const u8, len: usize),
        baseline = memcpy_baseline,
        alternatives = []
    }

    crate::alt_fn! {
        /// Dispatched kernel memzero.
        pub fn kernel_memzero(dst: *mut u8, len: usize),
        baseline = memzero_baseline,
        alternatives = []
    }

    crate::alt_fn! {
        /// Dispatched kernel memmove.
        pub fn kernel_memmove(dst: *mut u8, src: *const u8, len: usize),
        baseline = memmove_baseline,
        alternatives = []
    }

    crate::alt_fn! {
        /// Dispatched kernel memcmp.
        pub fn kernel_memcmp(a: *const u8, b: *const u8, len: usize) -> i32,
        baseline = memcmp_baseline,
        alternatives = []
    }
}

// ===========================================================================
// Compiler builtin symbols (kernel target only)
// ===========================================================================

/// `#[unsafe(no_mangle)]` symbols that replace `compiler_builtins`' `mem` feature.
///
/// These are automatically called by compiler-generated code for
/// `core::ptr::copy_nonoverlapping`, `core::ptr::write_bytes`, etc.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod builtins {
    use super::dispatch;

    /// Compiler builtin: copies `len` bytes from `src` to `dst`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn memcpy(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
        // SAFETY: Caller (compiler-generated code) guarantees valid,
        // non-overlapping regions.
        unsafe { dispatch::kernel_memcpy(dst, src, len) };
        dst
    }

    /// Compiler builtin: copies `len` bytes, handling overlap.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn memmove(dst: *mut u8, src: *const u8, len: usize) -> *mut u8 {
        // SAFETY: Caller guarantees valid regions.
        unsafe { dispatch::kernel_memmove(dst, src, len) };
        dst
    }

    /// Compiler builtin: fills `len` bytes at `dst` with `c`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn memset(dst: *mut u8, c: i32, len: usize) -> *mut u8 {
        if c == 0 {
            // SAFETY: Caller guarantees valid region.
            unsafe { dispatch::kernel_memzero(dst, len) };
        } else {
            // Use inline asm directly — core::ptr::write_bytes lowers to
            // a memset call, which would recurse back into this function.
            // SAFETY: Caller guarantees dst is valid for len-byte writes.
            unsafe {
                core::arch::asm!(
                    "rep stosb",
                    inout("rdi") dst => _,
                    inout("rcx") len => _,
                    inout("al") c as u8 => _,
                    options(nostack, preserves_flags),
                );
            }
        }
        dst
    }

    /// Compiler builtin: compares `len` bytes.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, len: usize) -> i32 {
        // SAFETY: Caller guarantees valid regions.
        unsafe { dispatch::kernel_memcmp(a, b, len) }
    }

    /// Compiler builtin: compares `len` bytes (BSD variant).
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn bcmp(a: *const u8, b: *const u8, len: usize) -> i32 {
        // SAFETY: Caller guarantees valid regions.
        unsafe { dispatch::kernel_memcmp(a, b, len) }
    }
}
