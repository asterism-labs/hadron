//! FPU/SSE/AVX enablement and kernel FPU state management.
//!
//! Provides two pieces:
//!
//! 1. **`enable_fpu_support()`** — Sets CR4 bits (OSFXSR, OSXMMEXCPT,
//!    OSXSAVE) and XCR0 to enable SSE/AVX instructions on the calling CPU.
//!    Called during boot for both BSP and each AP.
//!
//! 2. **`KernelFpuGuard`** — RAII guard that saves/restores FPU state and
//!    disables preemption (interrupts) so the kernel can safely use XMM/YMM
//!    registers for bulk operations.

#[cfg(hadron_kernel_fpu)]
use core::cell::UnsafeCell;

use super::cpuid::{self, CpuFeatures};
use super::registers::control::{Cr4, Cr4Flags};
use hadron_arch_x86_64::registers::xcr0::{self, Xcr0Flags};

// ---------------------------------------------------------------------------
// FPU enablement (called per-CPU during boot)
// ---------------------------------------------------------------------------

/// Enables FPU/SSE/AVX support on the calling CPU.
///
/// Sets CR4.OSFXSR + CR4.OSXMMEXCPT (required for FXSAVE/FXRSTOR, SSE,
/// and proper #XM exception routing), and if the CPU supports XSAVE, also
/// sets CR4.OSXSAVE and configures XCR0 to enable SSE state (and AVX state
/// if available).
///
/// # Safety
///
/// Must be called after [`cpuid::init()`](super::cpuid::init) on the BSP,
/// or after [`cpuid::verify_ap()`](super::cpuid::verify_ap) on APs.
pub unsafe fn enable_fpu_support() {
    let features = cpuid::cpu_features();

    if !features.contains(CpuFeatures::SSE2) {
        // SSE2 is mandatory on x86_64, but be defensive.
        return;
    }

    // Enable FXSAVE/FXRSTOR (required for SSE state save/restore) and
    // SIMD floating-point exception handling (#XM instead of #UD).
    let mut cr4 = Cr4::read();
    cr4 |= Cr4Flags::OSFXSR | Cr4Flags::OSXMMEXCPT;

    if features.contains(CpuFeatures::XSAVE) {
        // Enable XSAVE family of instructions.
        cr4 |= Cr4Flags::OSXSAVE;
        // SAFETY: CR4 flags are valid — we just added OSFXSR/OSXMMEXCPT/OSXSAVE.
        unsafe { Cr4::write(cr4) };

        // Configure XCR0: enable x87 + SSE.
        // SAFETY: CR4.OSXSAVE is now set.
        let mut xcr0_val = unsafe { xcr0::xgetbv() };
        xcr0_val |= Xcr0Flags::X87 | Xcr0Flags::SSE;

        if features.contains(CpuFeatures::AVX) {
            xcr0_val |= Xcr0Flags::AVX;
        }

        // SAFETY: CR4.OSXSAVE is set and the flags are valid for this CPU.
        unsafe { xcr0::xsetbv(xcr0_val) };

        // Validate that the XSAVE area fits in our static buffer.
        #[cfg(hadron_kernel_fpu)]
        {
            let xsave_size = cpuid::cpuid_sub(0xD, 0).ebx as usize;
            assert!(
                xsave_size <= FPU_SAVE_AREA_SIZE,
                "XSAVE area ({} bytes) exceeds FPU_SAVE_AREA_SIZE ({})",
                xsave_size,
                FPU_SAVE_AREA_SIZE,
            );
        }
    } else {
        // SAFETY: CR4 flags are valid — we just added OSFXSR/OSXMMEXCPT.
        unsafe { Cr4::write(cr4) };
    }
}

// ---------------------------------------------------------------------------
// Per-CPU FPU save area
// ---------------------------------------------------------------------------

/// Conservative FPU save area size.
///
/// FXSAVE requires 512 bytes; XSAVE with SSE+AVX needs ~832 bytes.
/// We use 1024 for headroom. Validated at boot against CPUID.0DH:EBX.
#[cfg(hadron_kernel_fpu)]
const FPU_SAVE_AREA_SIZE: usize = 1024;

/// Per-CPU FPU state save area, 64-byte aligned for XSAVE.
#[cfg(hadron_kernel_fpu)]
#[repr(C, align(64))]
pub struct FpuSaveArea {
    data: [u8; FPU_SAVE_AREA_SIZE],
}

#[cfg(hadron_kernel_fpu)]
impl FpuSaveArea {
    const fn new() -> Self {
        Self {
            data: [0u8; FPU_SAVE_AREA_SIZE],
        }
    }
}

/// Per-CPU FPU save areas. Each CPU has its own slot so `KernelFpuGuard`
/// can save/restore without heap allocation.
#[cfg(hadron_kernel_fpu)]
hadron_core::percpu_static!(static FPU_SAVE_AREAS: UnsafeCell<FpuSaveArea> =
    UnsafeCell::new(FpuSaveArea::new()));

// Debug-only nesting guard.
#[cfg(all(hadron_kernel_fpu, debug_assertions))]
hadron_core::percpu_static!(static FPU_DEPTH: hadron_core::sync::atomic::AtomicU32 =
    hadron_core::sync::atomic::AtomicU32::new(0));

// ---------------------------------------------------------------------------
// KernelFpuGuard
// ---------------------------------------------------------------------------

/// All managed XSAVE components (x87 + SSE + AVX).
#[cfg(hadron_kernel_fpu)]
const RFBM_ALL: u64 = 0xFFFF_FFFF_FFFF_FFFF;

/// RAII guard that saves the current FPU state and disables interrupts,
/// allowing the kernel to use XMM/YMM registers safely.
///
/// On drop, the FPU state is restored and the previous interrupt state
/// is reinstated.
///
/// # Usage
///
/// ```ignore
/// let _fpu = KernelFpuGuard::new();
/// // Safe to use SSE/AVX intrinsics here.
/// // State is restored when `_fpu` is dropped.
/// ```
#[cfg(hadron_kernel_fpu)]
pub struct KernelFpuGuard {
    irq_was_enabled: bool,
}

#[cfg(hadron_kernel_fpu)]
impl KernelFpuGuard {
    /// Saves FPU state and disables interrupts.
    pub fn new() -> Self {
        use super::instructions::interrupts;
        use hadron_arch_x86_64::instructions::misc;

        let irq_was_enabled = interrupts::are_enabled();
        if irq_was_enabled {
            interrupts::disable();
        }

        #[cfg(debug_assertions)]
        {
            let depth = FPU_DEPTH.get();
            let prev = depth.fetch_add(1, hadron_core::sync::atomic::Ordering::Relaxed);
            debug_assert!(prev == 0, "KernelFpuGuard nested (depth={})", prev + 1);
        }

        // Save current FPU state to the per-CPU buffer.
        let area = FPU_SAVE_AREAS.get();
        let ptr = area.get().cast::<u8>();

        if cpuid::has_feature(CpuFeatures::XSAVE) {
            // SAFETY: ptr is 64-byte aligned (FpuSaveArea is repr(align(64))),
            // points to FPU_SAVE_AREA_SIZE bytes, and CR4.OSXSAVE is set.
            // Interrupts are disabled so no concurrent FPU use.
            unsafe { misc::xsave64(ptr, RFBM_ALL) };
        } else {
            // SAFETY: ptr is 64-byte aligned (exceeds 16-byte requirement),
            // points to at least 512 bytes. Interrupts are disabled.
            unsafe { misc::fxsave64(ptr) };
        }

        Self { irq_was_enabled }
    }
}

#[cfg(hadron_kernel_fpu)]
impl Drop for KernelFpuGuard {
    fn drop(&mut self) {
        use hadron_arch_x86_64::instructions::misc;

        // Restore FPU state from the per-CPU buffer.
        let area = FPU_SAVE_AREAS.get();
        let ptr = area.get().cast::<u8>();

        if cpuid::has_feature(CpuFeatures::XSAVE) {
            // SAFETY: ptr is 64-byte aligned, points to valid XSAVE data,
            // and CR4.OSXSAVE is set. Interrupts are still disabled.
            unsafe { misc::xrstor64(ptr.cast_const(), RFBM_ALL) };
        } else {
            // SAFETY: ptr is 64-byte aligned, points to valid FXSAVE data.
            // Interrupts are still disabled.
            unsafe { misc::fxrstor64(ptr.cast_const()) };
        }

        #[cfg(debug_assertions)]
        {
            let depth = FPU_DEPTH.get();
            depth.fetch_sub(1, hadron_core::sync::atomic::Ordering::Relaxed);
        }

        if self.irq_was_enabled {
            // SAFETY: Interrupts were previously enabled, so re-enabling is safe.
            unsafe { super::instructions::interrupts::enable() };
        }
    }
}
