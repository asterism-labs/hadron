//! Control registers (CR0, CR2, CR3, CR4).

use crate::addr::PhysAddr;

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
        const PSE  = 1 << 4;
        /// Physical Address Extension.
        const PAE  = 1 << 5;
        /// Page Global Enable.
        const PGE  = 1 << 7;
        /// 57-bit linear addresses (5-level paging).
        const LA57 = 1 << 12;
    }
}

/// CR0 register.
pub struct Cr0;

impl Cr0 {
    /// Reads the current CR0 value.
    #[inline]
    pub fn read() -> Cr0Flags {
        let val: u64;
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
        unsafe {
            core::arch::asm!("mov cr4, {}", in(reg) flags.bits(), options(nostack, preserves_flags));
        }
    }
}
