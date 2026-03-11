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
