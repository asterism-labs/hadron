//! x86_64 page table structures.
//!
//! Provides types for manipulating 4-level page tables (PML4 -> PDPT -> PD -> PT).

use crate::addr::PhysAddr;

/// Physical address mask: bits 12..51 of a page table entry.
pub const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

bitflags::bitflags! {
    /// Page table entry flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PageTableFlags: u64 {
        /// Entry is present / valid.
        const PRESENT       = 1 << 0;
        /// Page is writable.
        const WRITABLE      = 1 << 1;
        /// Page is accessible from user mode (ring 3).
        const USER          = 1 << 2;
        /// Write-through caching.
        const WRITE_THROUGH = 1 << 3;
        /// Cache disabled.
        const CACHE_DISABLE = 1 << 4;
        /// PS bit -- 2 MiB page in PD, 1 GiB page in PDPT.
        const HUGE_PAGE     = 1 << 7;
        /// Global page (not flushed on CR3 switch when CR4.PGE is set).
        const GLOBAL        = 1 << 8;
        /// PAT bit for 2 MiB huge pages (bit 12). For 4 KiB pages, PAT is bit 7.
        const PAT_HUGE      = 1 << 12;
        /// No-execute bit (requires EFER.NXE).
        const NO_EXECUTE    = 1 << 63;
    }
}

bitflags::bitflags! {
    /// Page fault error code flags pushed by the CPU.
    ///
    /// Bits 0–4 describe the nature of the fault. The remaining bits are
    /// reserved or used by newer CPU features (SGX, shadow stacks) and are
    /// not decoded here.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PageFaultErrorCode: u64 {
        /// 1 = protection violation, 0 = not-present page.
        const PRESENT          = 1 << 0;
        /// 1 = write access caused the fault.
        const WRITE            = 1 << 1;
        /// 1 = fault occurred in user mode.
        const USER             = 1 << 2;
        /// 1 = a reserved bit was set in a page table entry.
        const RESERVED_WRITE   = 1 << 3;
        /// 1 = fault was caused by an instruction fetch.
        const INSTRUCTION_FETCH = 1 << 4;
    }
}

/// A single page table entry (64 bits).
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    /// An empty (not present) entry.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Creates an entry pointing to `phys_addr` with the given `flags`.
    pub const fn new(phys_addr: PhysAddr, flags: PageTableFlags) -> Self {
        Self((phys_addr.as_u64() & ADDR_MASK) | flags.bits())
    }

    /// Returns `true` if the PRESENT bit is set.
    pub const fn is_present(self) -> bool {
        self.0 & 1 != 0
    }

    /// Returns the physical address stored in this entry.
    pub const fn address(self) -> PhysAddr {
        // SAFETY: The masked value is guaranteed to fit in 52 bits.
        unsafe { PhysAddr::new_unchecked(self.0 & ADDR_MASK) }
    }

    /// Returns the flags portion of this entry.
    pub const fn flags(self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.0 & !ADDR_MASK)
    }
}

/// A 4 KiB-aligned page table containing 512 entries.
#[repr(C, align(4096))]
pub struct PageTable {
    /// The 512 entries of this page table.
    pub entries: [PageTableEntry; 512],
}

impl PageTable {
    /// Zero-initializes all entries.
    pub fn zero(&mut self) {
        self.entries.fill(PageTableEntry::empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::PhysAddr;

    #[test]
    fn empty_entry_not_present() {
        let entry = PageTableEntry::empty();
        assert!(!entry.is_present());
        assert_eq!(entry.address().as_u64(), 0);
    }

    #[test]
    fn entry_present_flag() {
        let entry = PageTableEntry::new(PhysAddr::new(0x1000), PageTableFlags::PRESENT);
        assert!(entry.is_present());
    }

    #[test]
    fn entry_address_masked() {
        let addr = PhysAddr::new(0x0000_1234_5000);
        let entry = PageTableEntry::new(addr, PageTableFlags::PRESENT);
        assert_eq!(entry.address().as_u64(), 0x0000_1234_5000);
    }

    #[test]
    fn flags_roundtrip() {
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
        let entry = PageTableEntry::new(PhysAddr::new(0x2000), flags);
        let got = entry.flags();
        assert!(got.contains(PageTableFlags::PRESENT));
        assert!(got.contains(PageTableFlags::WRITABLE));
        assert!(got.contains(PageTableFlags::NO_EXECUTE));
        assert!(!got.contains(PageTableFlags::USER));
    }

    #[test]
    fn address_does_not_leak_flags() {
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
        let entry = PageTableEntry::new(PhysAddr::new(0x3000), flags);
        // Address should only have bits 12..51.
        let addr = entry.address().as_u64();
        assert_eq!(addr, 0x3000);
        assert_eq!(addr & !ADDR_MASK, 0, "address leaked flag bits");
    }

    #[test]
    fn flags_do_not_leak_address() {
        let entry = PageTableEntry::new(
            PhysAddr::new(0x000F_FFFF_FFFF_F000),
            PageTableFlags::PRESENT,
        );
        let flags_bits = entry.flags().bits();
        // Flags should not contain any address bits.
        assert_eq!(flags_bits & ADDR_MASK, 0, "flags leaked address bits");
    }

    #[test]
    fn huge_page_flag() {
        let entry = PageTableEntry::new(
            PhysAddr::new(0x20_0000),
            PageTableFlags::PRESENT | PageTableFlags::HUGE_PAGE,
        );
        assert!(entry.flags().contains(PageTableFlags::HUGE_PAGE));
    }

    #[test]
    fn addr_mask_bit_range() {
        // ADDR_MASK should have bits 12..51 set and nothing else.
        for bit in 0..64 {
            let expected = (12..52).contains(&bit);
            let actual = (ADDR_MASK >> bit) & 1 == 1;
            assert_eq!(actual, expected, "bit {bit} mismatch in ADDR_MASK");
        }
    }

    #[test]
    fn page_fault_present() {
        let code = PageFaultErrorCode::from_bits_truncate(0b00001);
        assert!(code.contains(PageFaultErrorCode::PRESENT));
        assert!(!code.contains(PageFaultErrorCode::WRITE));
    }

    #[test]
    fn page_fault_write() {
        let code = PageFaultErrorCode::from_bits_truncate(0b00010);
        assert!(code.contains(PageFaultErrorCode::WRITE));
    }

    #[test]
    fn page_fault_user() {
        let code = PageFaultErrorCode::from_bits_truncate(0b00100);
        assert!(code.contains(PageFaultErrorCode::USER));
    }

    #[test]
    fn page_fault_instruction_fetch() {
        let code = PageFaultErrorCode::from_bits_truncate(0b10000);
        assert!(code.contains(PageFaultErrorCode::INSTRUCTION_FETCH));
        assert!(!code.contains(PageFaultErrorCode::PRESENT));
    }
}
