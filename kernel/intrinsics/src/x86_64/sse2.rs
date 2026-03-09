//! Raw SSE2 intrinsics via inline assembly.
//!
//! All functions are `unsafe` and require:
//! - The CPU supports SSE2 (always true on x86_64, but FPU must be enabled).
//! - The caller holds a `KernelFpuGuard`.
//!
//! This crate is compiled with `-Ctarget-feature=+sse2` via per-crate
//! `rustc_flags` in gluon, enabling the `xmm_reg` register class in
//! inline assembly. Functions use `extern "sysv64"` to prevent XMM
//! registers from leaking into the soft-float calling convention used
//! by the rest of the kernel.

use core::arch::asm;

/// Number of bytes in a single SSE2 register (128 bits).
pub const XMM_BYTES: usize = 16;

/// Minimum copy size where SSE2 unrolled copies beat `rep movsb`.
///
/// Below this threshold, the overhead of `KernelFpuGuard` (FXSAVE/XSAVE +
/// interrupt disable) is not worth it.
pub const SSE2_MEMCPY_THRESHOLD: usize = 128;

/// Minimum zero size where SSE2 beats `rep stosb`.
pub const SSE2_MEMZERO_THRESHOLD: usize = 128;

/// Copies 16 bytes from `src` to `dst` using unaligned SSE2 moves.
///
/// # Safety
///
/// - `dst` must be valid for 16-byte writes.
/// - `src` must be valid for 16-byte reads.
/// - Caller must hold a `KernelFpuGuard`.
pub unsafe extern "sysv64" fn copy_128_unaligned(dst: *mut u8, src: *const u8) {
    unsafe {
        asm!(
            "movdqu xmm0, [rsi]",
            "movdqu [rdi], xmm0",
            inout("rdi") dst => _,
            inout("rsi") src => _,
            out("xmm0") _,
            options(nostack, preserves_flags),
        );
    }
}

/// Zeroes 16 bytes at `dst` using SSE2.
///
/// # Safety
///
/// - `dst` must be valid for 16-byte writes.
/// - Caller must hold a `KernelFpuGuard`.
pub unsafe extern "sysv64" fn zero_128(dst: *mut u8) {
    unsafe {
        asm!(
            "pxor xmm0, xmm0",
            "movdqu [rdi], xmm0",
            inout("rdi") dst => _,
            out("xmm0") _,
            options(nostack, preserves_flags),
        );
    }
}

/// Issues a `prefetcht0` hint for the given address.
///
/// # Safety
///
/// - `addr` should point to readable memory (faults are suppressed by HW).
/// - Caller must hold a `KernelFpuGuard`.
pub unsafe extern "sysv64" fn prefetch_t0(addr: *const u8) {
    unsafe {
        asm!(
            "prefetcht0 [rdi]",
            in("rdi") addr,
            options(nostack, preserves_flags),
        );
    }
}

/// SSE2 memcpy inner loop: copies `len` bytes from `src` to `dst` using
/// 4x unrolled MOVDQU (64 bytes/iteration), with REP MOVSB for the tail.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
/// - `src` must be valid for `len`-byte reads.
/// - Memory regions must not overlap.
/// - CPU must support SSE2 and caller must hold a `KernelFpuGuard`.
pub unsafe extern "sysv64" fn memcpy_sse2_inner(dst: *mut u8, src: *const u8, len: usize) {
    // sysv64 ABI: dst in rdi, src in rsi, len in rdx
    unsafe {
        asm!(
            "    cmp rdx, 64",
            "    jb  3f",
            ".p2align 4",
            "2:",
            "    movdqu xmm0, [rsi]",
            "    movdqu xmm1, [rsi + 16]",
            "    movdqu xmm2, [rsi + 32]",
            "    movdqu xmm3, [rsi + 48]",
            "    movdqu [rdi], xmm0",
            "    movdqu [rdi + 16], xmm1",
            "    movdqu [rdi + 32], xmm2",
            "    movdqu [rdi + 48], xmm3",
            "    add rsi, 64",
            "    add rdi, 64",
            "    sub rdx, 64",
            "    cmp rdx, 64",
            "    jae 2b",
            "3:",
            "    mov rcx, rdx",
            "    rep movsb",
            inout("rdi") dst => _,
            inout("rsi") src => _,
            inout("rdx") len => _,
            out("rcx") _,
            out("xmm0") _,
            out("xmm1") _,
            out("xmm2") _,
            out("xmm3") _,
            options(nostack),
        );
    }
}

/// SSE2 memzero inner loop: zeroes `len` bytes at `dst` using 4x unrolled
/// MOVDQU of a zeroed XMM register (64 bytes/iteration), with `rep stosb`
/// for the tail.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
/// - CPU must support SSE2 and caller must hold a `KernelFpuGuard`.
pub unsafe extern "sysv64" fn memzero_sse2_inner(dst: *mut u8, len: usize) {
    // sysv64 ABI: dst in rdi, len in rsi
    unsafe {
        asm!(
            "pxor xmm0, xmm0",
            "    cmp rsi, 64",
            "    jb  3f",
            ".p2align 4",
            "2:",
            "    movdqu [rdi], xmm0",
            "    movdqu [rdi + 16], xmm0",
            "    movdqu [rdi + 32], xmm0",
            "    movdqu [rdi + 48], xmm0",
            "    add rdi, 64",
            "    sub rsi, 64",
            "    cmp rsi, 64",
            "    jae 2b",
            "3:",
            "    mov rcx, rsi",
            "    xor eax, eax",
            "    rep stosb",
            inout("rdi") dst => _,
            inout("rsi") len => _,
            out("rcx") _,
            out("rax") _,
            out("xmm0") _,
            options(nostack),
        );
    }
}

/// SSE2 memmove inner loop: copies `len` bytes from `src` to `dst`,
/// correctly handling overlapping regions.
///
/// Uses forward 4x unrolled MOVDQU when `dst <= src` (or no overlap),
/// and backward single-register MOVDQU when `dst > src`. Tails are
/// handled byte-by-byte.
///
/// # Safety
///
/// - `dst` must be valid for `len`-byte writes.
/// - `src` must be valid for `len`-byte reads.
/// - CPU must support SSE2 and caller must hold a `KernelFpuGuard`.
pub unsafe extern "sysv64" fn memmove_sse2_inner(dst: *mut u8, src: *const u8, len: usize) {
    // sysv64 ABI: dst in rdi, src in rsi, len in rdx
    unsafe {
        asm!(
            // If dst <= src, forward copy is safe.
            "    cmp rdi, rsi",
            "    jbe 20f",
            // dst > src: check if regions overlap (dst < src + len).
            "    mov rax, rsi",
            "    add rax, rdx",
            "    cmp rdi, rax",
            "    jae 20f",

            // ── Backward copy (dst > src, overlapping) ──
            // Start from the end of both buffers.
            "    lea rdi, [rdi + rdx - 16]",
            "    lea rsi, [rsi + rdx - 16]",
            // Handle 16-byte chunks backward.
            "    cmp rdx, 16",
            "    jb  6f",
            ".p2align 4",
            "5:",
            "    movdqu xmm0, [rsi]",
            "    movdqu [rdi], xmm0",
            "    sub rsi, 16",
            "    sub rdi, 16",
            "    sub rdx, 16",
            "    cmp rdx, 16",
            "    jae 5b",
            "6:",
            // Byte-by-byte tail (backward). rdi/rsi point 16 bytes before
            // the remaining tail start; adjust to point at last byte.
            "    add rdi, 15",
            "    add rsi, 15",
            "    test rdx, rdx",
            "    jz  9f",
            "7:",
            "    mov al, [rsi]",
            "    mov [rdi], al",
            "    dec rsi",
            "    dec rdi",
            "    dec rdx",
            "    jnz 7b",
            "9:",
            "    jmp 99f",

            // ── Forward copy ──
            "20:",
            "    cmp rdx, 64",
            "    jb  23f",
            ".p2align 4",
            "22:",
            "    movdqu xmm0, [rsi]",
            "    movdqu xmm1, [rsi + 16]",
            "    movdqu xmm2, [rsi + 32]",
            "    movdqu xmm3, [rsi + 48]",
            "    movdqu [rdi], xmm0",
            "    movdqu [rdi + 16], xmm1",
            "    movdqu [rdi + 32], xmm2",
            "    movdqu [rdi + 48], xmm3",
            "    add rsi, 64",
            "    add rdi, 64",
            "    sub rdx, 64",
            "    cmp rdx, 64",
            "    jae 22b",
            "23:",
            // Forward tail: rep movsb.
            "    mov rcx, rdx",
            "    rep movsb",
            "99:",
            inout("rdi") dst => _,
            inout("rsi") src => _,
            inout("rdx") len => _,
            out("rax") _,
            out("rcx") _,
            out("xmm0") _,
            out("xmm1") _,
            out("xmm2") _,
            out("xmm3") _,
            options(nostack),
        );
    }
}

/// SSE2 memcmp inner loop: compares `len` bytes at `a` and `b`.
///
/// Returns 0 if equal, negative if `a < b`, positive if `a > b`
/// (based on the first differing byte).
///
/// Uses 16-byte PCMPEQB + PMOVMSKB chunks, with BSF to find the first
/// mismatch, and byte-by-byte comparison for the tail.
///
/// # Safety
///
/// - `a` must be valid for `len`-byte reads.
/// - `b` must be valid for `len`-byte reads.
/// - CPU must support SSE2 and caller must hold a `KernelFpuGuard`.
pub unsafe extern "sysv64" fn memcmp_sse2_inner(a: *const u8, b: *const u8, len: usize) -> i32 {
    // sysv64 ABI: a in rdi, b in rsi, len in rdx; return in eax
    let result: i32;
    unsafe {
        asm!(
            "    xor eax, eax",
            "    cmp rdx, 16",
            "    jb  5f",
            ".p2align 4",
            "2:",
            "    movdqu xmm0, [rdi]",
            "    movdqu xmm1, [rsi]",
            "    pcmpeqb xmm0, xmm1",
            "    pmovmskb ecx, xmm0",
            "    xor ecx, 0xFFFF",    // invert: 1 bits = mismatches
            "    jnz 4f",             // found a mismatch
            "    add rdi, 16",
            "    add rsi, 16",
            "    sub rdx, 16",
            "    cmp rdx, 16",
            "    jae 2b",
            // Fall through to byte tail.
            "5:",
            "    test rdx, rdx",
            "    jz  8f",
            "6:",
            "    movzx eax, byte ptr [rdi]",
            "    movzx ecx, byte ptr [rsi]",
            "    sub eax, ecx",
            "    jnz 9f",
            "    inc rdi",
            "    inc rsi",
            "    dec rdx",
            "    jnz 6b",
            "8:",
            "    xor eax, eax",
            "    jmp 9f",
            // Mismatch in 16-byte chunk: find first differing byte.
            "4:",
            "    bsf ecx, ecx",       // index of first mismatch
            "    movzx eax, byte ptr [rdi + rcx]",
            "    movzx ecx, byte ptr [rsi + rcx]",
            "    sub eax, ecx",
            "9:",
            inout("rdi") a => _,
            inout("rsi") b => _,
            inout("rdx") len => _,
            out("rcx") _,
            lateout("eax") result,
            out("xmm0") _,
            out("xmm1") _,
            options(nostack),
        );
    }
    result
}
