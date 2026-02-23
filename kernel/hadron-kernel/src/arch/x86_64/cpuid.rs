//! CPUID feature detection.
//!
//! Reads CPUID leaves at boot to build a [`CpuFeatures`] bitfield describing
//! the instruction-set extensions available on the running CPU. The BSP
//! detects once; APs verify they are a superset (homogeneous assumption
//! required by the alt-instruction patching engine).

use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Raw CPUID wrappers
// ---------------------------------------------------------------------------

/// Result of a CPUID instruction.
#[derive(Debug, Clone, Copy)]
pub struct CpuidResult {
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// Executes CPUID with the given leaf (EAX), sub-leaf ECX = 0.
#[inline]
pub fn cpuid(leaf: u32) -> CpuidResult {
    // RBX is reserved by LLVM for PIC, so we save/restore it manually.
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32);
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx_out:e}, ebx",
            "pop rbx",
            inout("eax") leaf => eax,
            ebx_out = out(reg) ebx,
            inout("ecx") 0u32 => ecx,
            out("edx") edx,
            options(nostack, preserves_flags),
        );
    }
    CpuidResult { eax, ebx, ecx, edx }
}

/// Executes CPUID with the given leaf (EAX) and sub-leaf (ECX).
#[inline]
pub fn cpuid_sub(leaf: u32, sub: u32) -> CpuidResult {
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32);
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx_out:e}, ebx",
            "pop rbx",
            inout("eax") leaf => eax,
            ebx_out = out(reg) ebx,
            inout("ecx") sub => ecx,
            out("edx") edx,
            options(nostack, preserves_flags),
        );
    }
    CpuidResult { eax, ebx, ecx, edx }
}

// ---------------------------------------------------------------------------
// CpuFeatures bitflags
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// CPU feature flags detected via CPUID.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CpuFeatures: u64 {
        // -- Leaf 1, ECX --
        /// SSE3 (Streaming SIMD Extensions 3).
        const SSE3      = 1 << 0;
        /// SSSE3 (Supplemental SSE3).
        const SSSE3     = 1 << 1;
        /// SSE4.1.
        const SSE4_1    = 1 << 2;
        /// SSE4.2.
        const SSE4_2    = 1 << 3;
        /// POPCNT instruction.
        const POPCNT    = 1 << 4;
        /// XSAVE/XRSTOR/XGETBV/XSETBV.
        const XSAVE     = 1 << 5;
        /// AVX (Advanced Vector Extensions).
        const AVX       = 1 << 6;

        // -- Leaf 1, EDX --
        /// SSE2 (baseline on all x86_64 CPUs).
        const SSE2      = 1 << 8;

        // -- Leaf 7, sub-leaf 0, EBX --
        /// AVX2 (256-bit integer SIMD).
        const AVX2      = 1 << 16;
        /// BMI1 (Bit Manipulation Instruction Set 1).
        const BMI1      = 1 << 17;
        /// BMI2 (Bit Manipulation Instruction Set 2).
        const BMI2      = 1 << 18;
        /// ERMS (Enhanced REP MOVSB/STOSB).
        const ERMS      = 1 << 19;

        // -- Leaf 1, ECX (virtualisation) --
        /// VMX (Virtual Machine Extensions).
        const VMX       = 1 << 24;

        // -- Extended leaf 0x8000_0001, EDX --
        /// NX (No-Execute) bit support.
        const NX        = 1 << 32;
        /// 1 GiB pages (PDPE1GB).
        const PDPE1GB   = 1 << 33;

        // -- Platform (set from ACPI, not CPUID) --
        /// IOMMU detected via ACPI DMAR/IVRS table.
        const IOMMU     = 1 << 48;
    }
}

// ---------------------------------------------------------------------------
// Global storage
// ---------------------------------------------------------------------------

/// BSP-detected features, set once during `init()`.
static CPU_FEATURES: AtomicU64 = AtomicU64::new(0);

/// Returns the detected CPU features. Must be called after [`init`].
#[inline]
pub fn cpu_features() -> CpuFeatures {
    CpuFeatures::from_bits_truncate(CPU_FEATURES.load(Ordering::Acquire))
}

/// Returns `true` if all flags in `f` are present.
#[inline]
pub fn has_feature(f: CpuFeatures) -> bool {
    cpu_features().contains(f)
}

/// Sets the IOMMU flag after ACPI DMAR/IVRS parsing.
pub fn set_iommu_present() {
    CPU_FEATURES.fetch_or(CpuFeatures::IOMMU.bits(), Ordering::Release);
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Reads CPUID leaves and returns the feature set of the running CPU.
fn detect() -> CpuFeatures {
    let mut features = CpuFeatures::empty();

    // Leaf 0: max standard leaf.
    let leaf0 = cpuid(0);
    let max_std = leaf0.eax;

    // Leaf 1: basic feature bits.
    if max_std >= 1 {
        let leaf1 = cpuid(1);

        // ECX bits
        if leaf1.ecx & (1 << 0) != 0 {
            features |= CpuFeatures::SSE3;
        }
        if leaf1.ecx & (1 << 5) != 0 {
            features |= CpuFeatures::VMX;
        }
        if leaf1.ecx & (1 << 9) != 0 {
            features |= CpuFeatures::SSSE3;
        }
        if leaf1.ecx & (1 << 19) != 0 {
            features |= CpuFeatures::SSE4_1;
        }
        if leaf1.ecx & (1 << 20) != 0 {
            features |= CpuFeatures::SSE4_2;
        }
        if leaf1.ecx & (1 << 23) != 0 {
            features |= CpuFeatures::POPCNT;
        }
        if leaf1.ecx & (1 << 26) != 0 {
            features |= CpuFeatures::XSAVE;
        }
        if leaf1.ecx & (1 << 28) != 0 {
            features |= CpuFeatures::AVX;
        }

        // EDX bits
        if leaf1.edx & (1 << 26) != 0 {
            features |= CpuFeatures::SSE2;
        }
    }

    // Leaf 7, sub-leaf 0: extended features.
    if max_std >= 7 {
        let leaf7 = cpuid_sub(7, 0);

        if leaf7.ebx & (1 << 3) != 0 {
            features |= CpuFeatures::BMI1;
        }
        if leaf7.ebx & (1 << 5) != 0 {
            features |= CpuFeatures::AVX2;
        }
        if leaf7.ebx & (1 << 8) != 0 {
            features |= CpuFeatures::BMI2;
        }
        if leaf7.ebx & (1 << 9) != 0 {
            features |= CpuFeatures::ERMS;
        }
    }

    // Extended leaf 0x8000_0001: AMD-style extended features.
    let ext0 = cpuid(0x8000_0000);
    if ext0.eax >= 0x8000_0001 {
        let ext1 = cpuid(0x8000_0001);

        if ext1.edx & (1 << 20) != 0 {
            features |= CpuFeatures::NX;
        }
        if ext1.edx & (1 << 26) != 0 {
            features |= CpuFeatures::PDPE1GB;
        }
    }

    features
}

// ---------------------------------------------------------------------------
// Boot integration
// ---------------------------------------------------------------------------

/// Detects CPU features on the BSP and stores them globally.
///
/// Called once from `cpu_init()` on the bootstrap processor.
pub fn init() {
    let features = detect();
    CPU_FEATURES.store(features.bits(), Ordering::Release);
    crate::kinfo!("CPUID: {:#x} ({:?})", features.bits(), features);
}

/// Verifies that the calling AP supports at least the BSP's feature set.
///
/// Panics if the AP is missing any feature the BSP detected, since the
/// alt-instruction patching assumes homogeneous CPUs.
pub fn verify_ap() {
    let bsp = cpu_features();
    let ap = detect();
    let missing = bsp.difference(ap);
    assert!(
        missing.is_empty(),
        "AP missing BSP features: {:?}",
        missing,
    );
}
