//! Control registers (CR0, CR2, CR3, CR4).
//!
//! # References
//!
//! - Intel SDM Vol. 3A, §2.5: Control Registers (CR0–CR4 flag definitions)
//!   <https://www.intel.com/content/www/us/en/developer/articles/technical/intel-sdm.html>
//! - OSDev Wiki: Control Register 0 / CR4
//!   <https://wiki.osdev.org/CPU_Registers_x86#CR0>

use crate::PhysAddr;

bitflags::bitflags! {
    /// CR0 register flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Cr0Flags: u64 {
        /// Protected mode enable.
        const PROTECTED_MODE = 1 << 0;
        /// Write protect.
        const WRITE_PROTECT  = 1 << 16;
        /// Paging enable.
        const PAGING         = 1 << 31;
    }
}

bitflags::bitflags! {
    /// CR4 register flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Cr4Flags: u64 {
        /// Page Size Extensions.
        const PSE     = 1 << 4;
        /// Physical Address Extension.
        const PAE     = 1 << 5;
        /// Page Global Enable.
        const PGE     = 1 << 7;
        /// FXSAVE/FXRSTOR support (enables SSE/SSE2 in kernel).
        const OSFXSR     = 1 << 9;
        /// OS handles SIMD floating-point exceptions (#XM, vector 19).
        const OSXMMEXCPT = 1 << 10;
        /// 57-bit linear addresses (5-level paging).
        const LA57       = 1 << 12;
        /// XSAVE/XRSTOR and XGETBV/XSETBV support.
        const OSXSAVE    = 1 << 18;
    }
}

/// CR0 register.
pub struct Cr0;

impl Cr0 {
    /// Reads the current CR0 value.
    #[inline]
    pub fn read() -> Cr0Flags {
        let val: u64;
        // SAFETY: Reading CR0 has no side effects.
        unsafe {
            core::arch::asm!("mov {}, cr0", out(reg) val, options(nomem, nostack, preserves_flags));
        }
        Cr0Flags::from_bits_truncate(val)
    }

    /// Writes a new value to CR0.
    ///
    /// # Safety
    ///
    /// Changing CR0 flags can affect CPU operation mode.
    #[inline]
    pub unsafe fn write(flags: Cr0Flags) {
        // SAFETY: Caller guarantees the new flags are valid.
        unsafe {
            core::arch::asm!("mov cr0, {}", in(reg) flags.bits(), options(nostack, preserves_flags));
        }
    }
}

/// CR2 register (page fault linear address).
pub struct Cr2;

impl Cr2 {
    /// Reads the page fault linear address from CR2.
    #[inline]
    pub fn read() -> u64 {
        let val: u64;
        // SAFETY: Reading CR2 has no side effects.
        unsafe {
            core::arch::asm!("mov {}, cr2", out(reg) val, options(nomem, nostack, preserves_flags));
        }
        val
    }
}

/// CR3 register (page table root).
pub struct Cr3;

impl Cr3 {
    /// Reads the current page table root physical address from CR3.
    #[inline]
    pub fn read() -> PhysAddr {
        let val: u64;
        // SAFETY: Reading CR3 has no side effects.
        unsafe {
            core::arch::asm!("mov {}, cr3", out(reg) val, options(nomem, nostack, preserves_flags));
        }
        PhysAddr::new_truncate(val)
    }

    /// Writes a new page table root physical address to CR3.
    ///
    /// # Safety
    ///
    /// The caller must ensure `addr` points to a valid, correctly-mapped
    /// PML4 page table.
    #[inline]
    pub unsafe fn write(addr: PhysAddr) {
        // SAFETY: Caller guarantees the address points to a valid PML4.
        unsafe {
            core::arch::asm!("mov cr3, {}", in(reg) addr.as_u64(), options(nostack, preserves_flags));
        }
    }
}

/// CR4 register.
pub struct Cr4;

impl Cr4 {
    /// Reads the current CR4 value.
    #[inline]
    pub fn read() -> Cr4Flags {
        let val: u64;
        // SAFETY: Reading CR4 has no side effects.
        unsafe {
            core::arch::asm!("mov {}, cr4", out(reg) val, options(nomem, nostack, preserves_flags));
        }
        Cr4Flags::from_bits_truncate(val)
    }

    /// Writes a new value to CR4.
    ///
    /// # Safety
    ///
    /// Changing CR4 flags can affect CPU operation mode.
    #[inline]
    pub unsafe fn write(flags: Cr4Flags) {
        // SAFETY: Caller guarantees the new flags are valid.
        unsafe {
            core::arch::asm!("mov cr4, {}", in(reg) flags.bits(), options(nostack, preserves_flags));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cr0_flags_construction() {
        let flags = Cr0Flags::PROTECTED_MODE | Cr0Flags::PAGING;
        assert!(flags.contains(Cr0Flags::PROTECTED_MODE));
        assert!(flags.contains(Cr0Flags::PAGING));
        assert!(!flags.contains(Cr0Flags::WRITE_PROTECT));
        assert_eq!(flags.bits(), (1 << 0) | (1 << 31));
    }

    #[test]
    fn cr4_flags_construction() {
        let flags = Cr4Flags::PAE | Cr4Flags::PGE;
        assert!(flags.contains(Cr4Flags::PAE));
        assert!(flags.contains(Cr4Flags::PGE));
        assert!(!flags.contains(Cr4Flags::PSE));
        assert_eq!(flags.bits(), (1 << 5) | (1 << 7));
    }

    #[test]
    fn cr0_flags_empty() {
        let flags = Cr0Flags::empty();
        assert_eq!(flags.bits(), 0);
        assert!(!flags.contains(Cr0Flags::PAGING));
    }

    #[test]
    fn cr4_flags_all_distinct() {
        // Verify no two flags share the same bit.
        let all = [
            Cr4Flags::PSE,
            Cr4Flags::PAE,
            Cr4Flags::PGE,
            Cr4Flags::OSFXSR,
            Cr4Flags::OSXMMEXCPT,
            Cr4Flags::LA57,
            Cr4Flags::OSXSAVE,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(!a.intersects(*b), "CR4 flags overlap: {a:?} and {b:?}");
            }
        }
    }
}
