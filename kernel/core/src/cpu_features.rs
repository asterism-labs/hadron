//! CPU feature flags for alt-function dispatch.
//!
//! The [`CpuFeatures`] bitfield describes instruction-set extensions
//! available on the running CPU. Detection lives in `hadron-kernel`;
//! this crate only defines the data type so that subsystem crates can
//! reference feature flags without depending on the kernel.

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
