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

// ---------------------------------------------------------------------------
// XCR0 helpers
// ---------------------------------------------------------------------------

/// Reads XCR0 (Extended Control Register 0) via XGETBV with ECX=0.
#[inline]
unsafe fn xgetbv() -> u64 {
    let (lo, hi): (u32, u32);
    unsafe {
        core::arch::asm!(
            "xgetbv",
            in("ecx") 0u32,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }
    (hi as u64) << 32 | lo as u64
}

/// Writes XCR0 via XSETBV with ECX=0.
#[inline]
unsafe fn xsetbv(val: u64) {
    let lo = val as u32;
    let hi = (val >> 32) as u32;
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
        unsafe { Cr4::write(cr4) };

        // Configure XCR0: enable x87 (bit 0) + SSE (bit 1).
        let mut xcr0 = unsafe { xgetbv() };
        xcr0 |= 0x1 | 0x2; // x87 + SSE/XMM

        if features.contains(CpuFeatures::AVX) {
            xcr0 |= 0x4; // AVX/YMM upper halves
        }

        unsafe { xsetbv(xcr0) };

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

#[cfg(hadron_kernel_fpu)]
use crate::percpu::{CpuLocal, MAX_CPUS};

/// Per-CPU FPU save areas. Each CPU has its own slot so `KernelFpuGuard`
/// can save/restore without heap allocation.
#[cfg(hadron_kernel_fpu)]
static FPU_SAVE_AREAS: CpuLocal<UnsafeCell<FpuSaveArea>> =
    CpuLocal::new([const { UnsafeCell::new(FpuSaveArea::new()) }; MAX_CPUS]);

// Debug-only nesting guard.
#[cfg(all(hadron_kernel_fpu, debug_assertions))]
static FPU_DEPTH: CpuLocal<core::sync::atomic::AtomicU32> =
    CpuLocal::new([const { core::sync::atomic::AtomicU32::new(0) }; MAX_CPUS]);

// ---------------------------------------------------------------------------
// KernelFpuGuard
// ---------------------------------------------------------------------------

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

        let irq_was_enabled = interrupts::are_enabled();
        if irq_was_enabled {
            interrupts::disable();
        }

        #[cfg(debug_assertions)]
        {
            let depth = FPU_DEPTH.get();
            let prev = depth.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            debug_assert!(prev == 0, "KernelFpuGuard nested (depth={})", prev + 1);
        }

        // Save current FPU state to the per-CPU buffer.
        let area = FPU_SAVE_AREAS.get();
        let ptr = area.get();

        if cpuid::has_feature(CpuFeatures::XSAVE) {
            // XSAVE64 with RFBM = all managed components (x87 + SSE + AVX).
            unsafe {
                core::arch::asm!(
                    "xsave64 [{}]",
                    in(reg) ptr,
                    in("eax") 0xFFFF_FFFFu32,
                    in("edx") 0xFFFF_FFFFu32,
                    options(nostack),
                );
            }
        } else {
            // FXSAVE64 (SSE only, 512-byte area).
            unsafe {
                core::arch::asm!(
                    "fxsave64 [{}]",
                    in(reg) ptr,
                    options(nostack),
                );
            }
        }

        Self { irq_was_enabled }
    }
}

#[cfg(hadron_kernel_fpu)]
impl Drop for KernelFpuGuard {
    fn drop(&mut self) {
        // Restore FPU state from the per-CPU buffer.
        let area = FPU_SAVE_AREAS.get();
        let ptr = area.get();

        if cpuid::has_feature(CpuFeatures::XSAVE) {
            unsafe {
                core::arch::asm!(
                    "xrstor64 [{}]",
                    in(reg) ptr,
                    in("eax") 0xFFFF_FFFFu32,
                    in("edx") 0xFFFF_FFFFu32,
                    options(nostack),
                );
            }
        } else {
            unsafe {
                core::arch::asm!(
                    "fxrstor64 [{}]",
                    in(reg) ptr,
                    options(nostack),
                );
            }
        }

        #[cfg(debug_assertions)]
        {
            let depth = FPU_DEPTH.get();
            depth.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
        }

        if self.irq_was_enabled {
            unsafe { super::instructions::interrupts::enable() };
        }
    }
}
