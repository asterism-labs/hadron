//! CPUID instruction wrapper.
//!
//! Provides a safe interface to the `cpuid` instruction for querying
//! CPU features, topology, and other processor information.
//!
//! # References
//!
//! - Intel SDM Vol. 2A: CPUID — CPU Identification
//!   <https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html>

/// Result of a CPUID query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuidResult {
    /// EAX output.
    pub eax: u32,
    /// EBX output.
    pub ebx: u32,
    /// ECX output.
    pub ecx: u32,
    /// EDX output.
    pub edx: u32,
}

/// Executes the CPUID instruction with the given leaf (EAX).
///
/// ECX is set to 0. Use [`cpuid_sub`] to specify a sub-leaf.
#[inline]
pub fn cpuid(leaf: u32) -> CpuidResult {
    cpuid_sub(leaf, 0)
}

/// Executes the CPUID instruction with the given leaf (EAX) and sub-leaf (ECX).
#[inline]
pub fn cpuid_sub(leaf: u32, sub_leaf: u32) -> CpuidResult {
    // RBX is reserved by LLVM for PIC, so we save/restore it manually.
    let (eax, ebx, ecx, edx): (u32, u32, u32, u32);
    // SAFETY: CPUID is always valid and has no side effects. RBX is
    // saved/restored around the instruction because LLVM reserves it.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx_out:e}, ebx",
            "pop rbx",
            inout("eax") leaf => eax,
            ebx_out = out(reg) ebx,
            inout("ecx") sub_leaf => ecx,
            out("edx") edx,
            options(nostack, preserves_flags),
        );
    }
    CpuidResult { eax, ebx, ecx, edx }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn cpuid_leaf_0_vendor_string() {
        let result = cpuid(0);
        // Max leaf must be >= 1.
        assert!(result.eax >= 1, "CPUID max leaf too low: {}", result.eax);

        // Reconstruct 12-byte vendor string from ebx, edx, ecx.
        let mut vendor = [0u8; 12];
        vendor[0..4].copy_from_slice(&result.ebx.to_le_bytes());
        vendor[4..8].copy_from_slice(&result.edx.to_le_bytes());
        vendor[8..12].copy_from_slice(&result.ecx.to_le_bytes());
        let vendor_str = core::str::from_utf8(&vendor).expect("vendor not UTF-8");

        // Must be one of the well-known vendor strings.
        let known = [
            "GenuineIntel",
            "AuthenticAMD",
            "HygonGenuine",
            "GenuineTMx86",
        ];
        assert!(
            known.contains(&vendor_str),
            "unexpected CPUID vendor: {vendor_str:?}"
        );
    }

    #[test]
    fn cpuid_leaf_1_feature_bits() {
        let result = cpuid(1);
        // Long mode CPU must support FPU (bit 0) and SSE2 (bit 26) in EDX.
        assert!(result.edx & (1 << 0) != 0, "FPU not supported");
        assert!(result.edx & (1 << 26) != 0, "SSE2 not supported");
    }
}
