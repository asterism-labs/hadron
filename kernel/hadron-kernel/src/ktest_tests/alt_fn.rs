//! Alt-instruction system tests — CPUID detection, FPU guard, and dispatched
//! memory operation tests (memcpy, memzero, memmove, memcmp).

use hadron_ktest::kernel_test;

// ---------------------------------------------------------------------------
// CPUID detection tests
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_cpuid_detects_sse2() {
    // SSE2 is mandatory on all x86_64 CPUs.
    let features = crate::arch::x86_64::cpuid::cpu_features();
    assert!(
        features.contains(crate::arch::x86_64::cpuid::CpuFeatures::SSE2),
        "SSE2 must be present on x86_64"
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_cpuid_features_nonzero() {
    // The feature set should not be empty after init().
    let features = crate::arch::x86_64::cpuid::cpu_features();
    assert!(
        !features.is_empty(),
        "cpu_features() should not be empty after init"
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_cpuid_raw_leaf0() {
    // Leaf 0 must return a valid max leaf >= 1.
    let result = crate::arch::x86_64::cpuid::cpuid(0);
    assert!(
        result.eax >= 1,
        "CPUID leaf 0 max_leaf must be >= 1, got {}",
        result.eax
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_cpuid_verify_ap_succeeds_on_bsp() {
    // verify_ap() should not panic when called on the BSP (features are a
    // superset of themselves).
    crate::arch::x86_64::cpuid::verify_ap();
}

// ---------------------------------------------------------------------------
// FPU enablement tests
// ---------------------------------------------------------------------------

#[cfg(hadron_kernel_fpu)]
#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_fpu_cr4_osfxsr_set() {
    use crate::arch::x86_64::registers::control::{Cr4, Cr4Flags};
    let cr4 = Cr4::read();
    assert!(
        cr4.contains(Cr4Flags::OSFXSR),
        "CR4.OSFXSR must be set after enable_fpu_support()"
    );
}

#[cfg(hadron_kernel_fpu)]
#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_fpu_cr4_osxmmexcpt_set() {
    use crate::arch::x86_64::registers::control::{Cr4, Cr4Flags};
    let cr4 = Cr4::read();
    assert!(
        cr4.contains(Cr4Flags::OSXMMEXCPT),
        "CR4.OSXMMEXCPT must be set after enable_fpu_support()"
    );
}

// ---------------------------------------------------------------------------
// KernelFpuGuard tests
// ---------------------------------------------------------------------------

#[cfg(hadron_kernel_fpu)]
#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_fpu_guard_acquire_release() {
    // Guard should be acquirable and releasable without panic.
    let _guard = crate::arch::x86_64::fpu::KernelFpuGuard::new();
    // Interrupts should be disabled while guard is held.
    assert!(
        !crate::arch::x86_64::instructions::interrupts::are_enabled(),
        "interrupts must be disabled while KernelFpuGuard is held"
    );
}

#[cfg(hadron_kernel_fpu)]
#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_fpu_guard_restores_interrupts() {
    use crate::arch::x86_64::instructions::interrupts;

    let irq_before = interrupts::are_enabled();
    {
        let _guard = crate::arch::x86_64::fpu::KernelFpuGuard::new();
        // Guard active — interrupts disabled.
    }
    // Guard dropped — interrupts should be restored to previous state.
    let irq_after = interrupts::are_enabled();
    assert_eq!(
        irq_before, irq_after,
        "KernelFpuGuard must restore interrupt state on drop"
    );
}

// ---------------------------------------------------------------------------
// Alt-fn dispatch tests
// ---------------------------------------------------------------------------

#[cfg(hadron_alt_instructions)]
#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_alt_fn_entries_exist() {
    let entries = crate::arch::x86_64::alt_fn::alt_fn_entries();
    // With kernel_fpu enabled, at least the SSE2 memcpy entry should exist.
    // Without kernel_fpu, the baseline dispatch entries still exist.
    assert!(
        !entries.is_empty(),
        "alt_fn entries should not be empty with builtin mem dispatch"
    );
    let _ = entries; // suppress unused warning
}

// ---------------------------------------------------------------------------
// Memcpy dispatch tests
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcpy_small() {
    let src = [0xABu8; 32];
    let mut dst = [0u8; 32];
    unsafe {
        hadron_core::mem::dispatch::kernel_memcpy(dst.as_mut_ptr(), src.as_ptr(), src.len());
    }
    assert_eq!(dst, src, "small memcpy should copy correctly");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcpy_large() {
    // Large enough to trigger the SSE2 path (>= 128 bytes).
    extern crate alloc;
    let src: alloc::vec::Vec<u8> = (0..256).map(|i| i as u8).collect();
    let mut dst = alloc::vec![0u8; 256];
    unsafe {
        hadron_core::mem::dispatch::kernel_memcpy(dst.as_mut_ptr(), src.as_ptr(), src.len());
    }
    assert_eq!(dst, src, "large memcpy should copy correctly");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcpy_zero_length() {
    let src = [0xFFu8; 8];
    let mut dst = [0u8; 8];
    unsafe {
        hadron_core::mem::dispatch::kernel_memcpy(dst.as_mut_ptr(), src.as_ptr(), 0);
    }
    assert_eq!(dst, [0u8; 8], "zero-length memcpy should not modify dst");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcpy_unaligned() {
    // Test with unaligned source and destination.
    extern crate alloc;
    let src_buf = alloc::vec![0xCDu8; 200];
    let mut dst_buf = alloc::vec![0u8; 200];
    // Offset by 3 bytes to force unalignment.
    let len = 150;
    unsafe {
        hadron_core::mem::dispatch::kernel_memcpy(
            dst_buf[3..].as_mut_ptr(),
            src_buf[3..].as_ptr(),
            len,
        );
    }
    assert_eq!(
        &dst_buf[3..3 + len],
        &src_buf[3..3 + len],
        "unaligned memcpy should copy correctly"
    );
}

// ---------------------------------------------------------------------------
// Memzero dispatch tests
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memzero_small() {
    let mut buf = [0xFFu8; 32];
    unsafe { hadron_core::mem::dispatch::kernel_memzero(buf.as_mut_ptr(), buf.len()) };
    assert_eq!(buf, [0u8; 32], "small memzero should zero all bytes");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memzero_large() {
    extern crate alloc;
    let mut buf = alloc::vec![0xFFu8; 256];
    unsafe { hadron_core::mem::dispatch::kernel_memzero(buf.as_mut_ptr(), buf.len()) };
    assert!(
        buf.iter().all(|&b| b == 0),
        "large memzero should zero all bytes"
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memzero_zero_length() {
    let mut buf = [0xFFu8; 8];
    unsafe { hadron_core::mem::dispatch::kernel_memzero(buf.as_mut_ptr(), 0) };
    assert_eq!(
        buf, [0xFFu8; 8],
        "zero-length memzero should not modify buffer"
    );
}

// ---------------------------------------------------------------------------
// Memmove dispatch tests
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memmove_non_overlapping() {
    let src = [0xABu8; 64];
    let mut dst = [0u8; 64];
    unsafe {
        hadron_core::mem::dispatch::kernel_memmove(dst.as_mut_ptr(), src.as_ptr(), src.len());
    }
    assert_eq!(dst, src, "non-overlapping memmove should copy correctly");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memmove_overlapping_forward() {
    // Overlapping: copy buf[0..64] to buf[16..80].
    extern crate alloc;
    let mut buf = alloc::vec![0u8; 128];
    for i in 0..64 {
        buf[i] = i as u8;
    }
    let expected: alloc::vec::Vec<u8> = (0..64).map(|i| i as u8).collect();
    unsafe {
        hadron_core::mem::dispatch::kernel_memmove(buf.as_mut_ptr().add(16), buf.as_ptr(), 64);
    }
    assert_eq!(
        &buf[16..80],
        expected.as_slice(),
        "forward overlap memmove failed"
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memmove_overlapping_backward() {
    // Overlapping: copy buf[16..80] to buf[0..64].
    extern crate alloc;
    let mut buf = alloc::vec![0u8; 128];
    for i in 0..64 {
        buf[16 + i] = i as u8;
    }
    let expected: alloc::vec::Vec<u8> = (0..64).map(|i| i as u8).collect();
    unsafe {
        hadron_core::mem::dispatch::kernel_memmove(buf.as_mut_ptr(), buf.as_ptr().add(16), 64);
    }
    assert_eq!(
        &buf[0..64],
        expected.as_slice(),
        "backward overlap memmove failed"
    );
}

// ---------------------------------------------------------------------------
// Memcmp dispatch tests
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcmp_equal() {
    let a = [0xABu8; 64];
    let b = [0xABu8; 64];
    let result =
        unsafe { hadron_core::mem::dispatch::kernel_memcmp(a.as_ptr(), b.as_ptr(), a.len()) };
    assert_eq!(result, 0, "equal buffers should return 0");
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcmp_less() {
    let a = [0x10u8; 64];
    let b = [0x20u8; 64];
    let result =
        unsafe { hadron_core::mem::dispatch::kernel_memcmp(a.as_ptr(), b.as_ptr(), a.len()) };
    assert!(result < 0, "a < b should return negative, got {}", result);
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcmp_greater() {
    let a = [0x30u8; 64];
    let b = [0x20u8; 64];
    let result =
        unsafe { hadron_core::mem::dispatch::kernel_memcmp(a.as_ptr(), b.as_ptr(), a.len()) };
    assert!(result > 0, "a > b should return positive, got {}", result);
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_kernel_memcmp_zero_length() {
    let a = [0xFFu8; 8];
    let b = [0x00u8; 8];
    let result = unsafe { hadron_core::mem::dispatch::kernel_memcmp(a.as_ptr(), b.as_ptr(), 0) };
    assert_eq!(result, 0, "zero-length memcmp should return 0");
}

// ---------------------------------------------------------------------------
// FPU context infrastructure tests
// ---------------------------------------------------------------------------

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_user_fpu_context_ptr_initialized() {
    // The percpu user_fpu_context_ptr (GS:[64]) must be non-null after boot.
    // A null pointer here means the timer stub's fxsave64 will page fault
    // when preempting a userspace process.
    let percpu = crate::percpu::PerCpuState::current();
    assert!(
        percpu.user_fpu_context_ptr != 0,
        "user_fpu_context_ptr must be initialized (non-null) after boot"
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_user_fpu_save_area_alignment() {
    // FXSAVE64 requires a 16-byte aligned destination. Our UserFpuSaveArea
    // is 64-byte aligned (for XSAVE compatibility). Verify the actual
    // pointer returned by USER_FPU_CONTEXT meets the alignment requirement.
    let ptr = crate::proc::TrapContext::user_fpu_context_ptr();
    assert!(
        (ptr as usize) % 64 == 0,
        "USER_FPU_CONTEXT pointer must be 64-byte aligned, got {:#x}",
        ptr as usize
    );
}

#[cfg(hadron_kernel_fpu)]
#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_fxsave_fxrstor_round_trip() {
    // Verify fxsave64/fxrstor64 round-trips preserve XMM state.
    // We set xmm0 to a known pattern, save, clobber, restore, and verify.
    use crate::proc::UserFpuSaveArea;

    let _fpu = crate::arch::x86_64::fpu::KernelFpuGuard::new();

    #[repr(C, align(64))]
    struct AlignedSaveArea([u8; 512]);

    let mut save_area = AlignedSaveArea([0u8; 512]);
    let save_ptr = save_area.0.as_mut_ptr();

    // Load a known pattern into xmm0.
    let pattern: [u8; 16] = [
        0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD,
        0xEF,
    ];
    unsafe {
        core::arch::asm!(
            "movdqu xmm0, [{}]",
            in(reg) pattern.as_ptr(),
            out("xmm0") _,
            options(nostack),
        );
    }

    // Save FPU state.
    unsafe {
        core::arch::asm!("fxsave64 [{}]", in(reg) save_ptr, options(nostack));
    }

    // Clobber xmm0 with zeroes.
    unsafe {
        core::arch::asm!("pxor xmm0, xmm0", out("xmm0") _, options(nostack));
    }

    // Restore FPU state — xmm0 should have our pattern again.
    unsafe {
        core::arch::asm!("fxrstor64 [{}]", in(reg) save_ptr, options(nostack));
    }

    // Read xmm0 back.
    let mut result = [0u8; 16];
    unsafe {
        core::arch::asm!(
            "movdqu [{}], xmm0",
            in(reg) result.as_mut_ptr(),
            options(nostack),
        );
    }

    assert_eq!(
        result, pattern,
        "fxsave64/fxrstor64 round-trip should preserve xmm0"
    );
}
