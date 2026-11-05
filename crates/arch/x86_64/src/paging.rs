//! 4-level page tables: entries, their flags, and the 4 KiB table.
//!
//! The layouts are in the Intel SDM, Vol. 3A §5.5, Tables 5-15 to 5-20.
//! An entry at any level shares the low flag bits. Bit 7 (PS) in a PDPT or
//! PD entry makes it map a 1 GiB or 2 MiB page instead of pointing at the
//! next table. A leaf entry's physical address starts at bit 12 for 4 KiB
//! pages, bit 21 for 2 MiB and bit 30 for 1 GiB. Bit 12 of a huge page
//! entry is the PAT bit, so the address mask depends on the page size.

use core::fmt;
use core::ops::{BitOr, BitOrAssign};

use crate::addr::PhysAddr;

/// Number of entries in one table.
pub const ENTRIES: usize = 512;

/// A level of the 4-level page table tree, from the root down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The root table, which CR3 points at. Each entry covers 512 GiB.
    Pml4,
    /// Page directory pointer table. Each entry covers 1 GiB.
    Pdpt,
    /// Page directory. Each entry covers 2 MiB.
    Pd,
    /// Page table. Each entry maps one 4 KiB page.
    Pt,
}

impl Level {
    /// The bit position of this level's 9-bit index in a virtual address.
    #[must_use]
    pub const fn shift(self) -> u32 {
        match self {
            Self::Pml4 => 39,
            Self::Pdpt => 30,
            Self::Pd => 21,
            Self::Pt => 12,
        }
    }

    /// Bytes one entry at this level covers.
    #[must_use]
    pub const fn entry_size(self) -> u64 {
        1 << self.shift()
    }

    /// The level below, or `None` for the page table.
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Pml4 => Some(Self::Pdpt),
            Self::Pdpt => Some(Self::Pd),
            Self::Pd => Some(Self::Pt),
            Self::Pt => None,
        }
    }

    /// The page size an entry at this level maps when it is a leaf, or
    /// `None` for the PML4, which cannot map a page.
    #[must_use]
    pub const fn page_size(self) -> Option<PageSize> {
        match self {
            Self::Pml4 => None,
            Self::Pdpt => Some(PageSize::Size1G),
            Self::Pd => Some(PageSize::Size2M),
            Self::Pt => Some(PageSize::Size4K),
        }
    }
}

/// The size of a mapped page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PageSize {
    /// A 4 KiB page, mapped by a page table entry.
    Size4K,
    /// A 2 MiB page, mapped by a page directory entry with PS set.
    Size2M,
    /// A 1 GiB page, mapped by a PDPT entry with PS set. The CPU supports
    /// it only if CPUID.80000001H:EDX.Page1GB\[bit 26\] is set.
    Size1G,
}

impl PageSize {
    /// The page size in bytes.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Size4K => 1 << 12,
            Self::Size2M => 1 << 21,
            Self::Size1G => 1 << 30,
        }
    }

    /// The level whose entries map pages of this size.
    #[must_use]
    pub const fn level(self) -> Level {
        match self {
            Self::Size4K => Level::Pt,
            Self::Size2M => Level::Pd,
            Self::Size1G => Level::Pdpt,
        }
    }
}

/// Bytes in a 4 KiB page, the size of every table and of one frame.
pub const PAGE_SIZE: u64 = PageSize::Size4K.bytes();

/// Flag bits of a page table entry.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct PageTableFlags(u64);

impl PageTableFlags {
    /// No flags.
    pub const EMPTY: Self = Self(0);
    /// Bit 0 (P): the entry is in use.
    pub const PRESENT: Self = Self(1 << 0);
    /// Bit 1 (R/W): writes are allowed.
    pub const WRITABLE: Self = Self(1 << 1);
    /// Bit 2 (U/S): ring 3 may access the region.
    pub const USER: Self = Self(1 << 2);
    /// Bit 3 (PWT): page-level write-through.
    pub const WRITE_THROUGH: Self = Self(1 << 3);
    /// Bit 4 (PCD): page-level cache disable.
    pub const CACHE_DISABLE: Self = Self(1 << 4);
    /// Bit 5 (A): set by the CPU when the entry is used for a translation.
    pub const ACCESSED: Self = Self(1 << 5);
    /// Bit 6 (D): set by the CPU when the page is written. Leaf entries
    /// only.
    pub const DIRTY: Self = Self(1 << 6);
    /// Bit 7 (PS): a PDPT or PD entry maps a 1 GiB or 2 MiB page.
    pub const HUGE_PAGE: Self = Self(1 << 7);
    /// Bit 8 (G): with CR4.PGE set, a CR3 write does not flush the
    /// translation. Leaf entries only.
    pub const GLOBAL: Self = Self(1 << 8);
    /// Bit 63 (XD): instruction fetches are not allowed. Needs
    /// `IA32_EFER.NXE`, or the bit is reserved.
    pub const NO_EXECUTE: Self = Self(1 << 63);

    /// Every bit this type names.
    const ALL: u64 = 0x1ff | 1 << 63;

    /// The flags as bits.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// The named flags in `bits`. Other bits are dropped.
    #[must_use]
    pub const fn from_bits_truncate(bits: u64) -> Self {
        Self(bits & Self::ALL)
    }

    /// Whether every flag in `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `self` with the flags in `other` cleared.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// `self` with the flags in `other` set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOr for PageTableFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl BitOrAssign for PageTableFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl fmt::Debug for PageTableFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [(PageTableFlags, &str); 11] = [
            (PageTableFlags::PRESENT, "PRESENT"),
            (PageTableFlags::WRITABLE, "WRITABLE"),
            (PageTableFlags::USER, "USER"),
            (PageTableFlags::WRITE_THROUGH, "WRITE_THROUGH"),
            (PageTableFlags::CACHE_DISABLE, "CACHE_DISABLE"),
            (PageTableFlags::ACCESSED, "ACCESSED"),
            (PageTableFlags::DIRTY, "DIRTY"),
            (PageTableFlags::HUGE_PAGE, "HUGE_PAGE"),
            (PageTableFlags::GLOBAL, "GLOBAL"),
            (PageTableFlags::NO_EXECUTE, "NO_EXECUTE"),
            (PageTableFlags::EMPTY, ""),
        ];
        let mut first = true;
        for (flag, name) in NAMES {
            if flag != Self::EMPTY && self.contains(flag) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        if first {
            f.write_str("EMPTY")?;
        }
        Ok(())
    }
}

/// Bits 51:12, the address of a 4 KiB page or of the next table.
const ADDR_MASK_4K: u64 = 0x000f_ffff_ffff_f000;

/// One 8-byte page table entry.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    /// An entry that is not present.
    pub const UNUSED: Self = Self(0);

    /// An entry that points at the next table, at `table`.
    ///
    /// # Panics
    ///
    /// Panics if `table` is not 4 KiB aligned or `flags` has `HUGE_PAGE`.
    #[must_use]
    pub const fn table(table: PhysAddr, flags: PageTableFlags) -> Self {
        assert!(table.is_aligned(PAGE_SIZE), "table is not 4 KiB aligned");
        assert!(!flags.contains(PageTableFlags::HUGE_PAGE));
        Self(table.as_u64() | flags.union(PageTableFlags::PRESENT).bits())
    }

    /// A leaf entry that maps a page of `size` at `frame`. The entry sets
    /// `PRESENT` and, for 2 MiB and 1 GiB pages, `HUGE_PAGE`.
    ///
    /// # Panics
    ///
    /// Panics if `frame` is not aligned to `size`.
    #[must_use]
    pub const fn page(frame: PhysAddr, size: PageSize, flags: PageTableFlags) -> Self {
        assert!(
            frame.is_aligned(size.bytes()),
            "frame is not aligned to the page size"
        );
        let mut flags = flags.union(PageTableFlags::PRESENT);
        if !matches!(size, PageSize::Size4K) {
            flags = flags.union(PageTableFlags::HUGE_PAGE);
        }
        Self(frame.as_u64() | flags.bits())
    }

    /// Whether the entry is present.
    #[must_use]
    pub const fn is_present(self) -> bool {
        self.flags().contains(PageTableFlags::PRESENT)
    }

    /// Whether a present PDPT or PD entry maps a page. Meaningless for a
    /// PML4 entry, where bit 7 is reserved, and for a page table entry,
    /// where bit 7 is the PAT bit.
    #[must_use]
    pub const fn is_huge(self) -> bool {
        self.flags().contains(PageTableFlags::HUGE_PAGE)
    }

    /// The flag bits.
    #[must_use]
    pub const fn flags(self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.0)
    }

    /// The address of the next table, or of a 4 KiB page.
    #[must_use]
    pub const fn address(self) -> PhysAddr {
        PhysAddr::new(self.0 & ADDR_MASK_4K)
    }

    /// The address of the page a leaf entry of `size` maps. Masks out the
    /// PAT bit of huge page entries.
    #[must_use]
    pub const fn page_address(self, size: PageSize) -> PhysAddr {
        PhysAddr::new(self.0 & ADDR_MASK_4K & !(size.bytes() - 1))
    }

    /// The entry as bits.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_present() {
            write!(
                f,
                "PageTableEntry({:#x}, {:?})",
                self.address(),
                self.flags()
            )
        } else {
            f.write_str("PageTableEntry(UNUSED)")
        }
    }
}

/// One 4 KiB table at any level.
#[derive(Clone)]
#[repr(C, align(4096))]
pub struct PageTable {
    /// The entries, indexed by [`VirtAddr::table_index`](crate::VirtAddr::table_index).
    pub entries: [PageTableEntry; ENTRIES],
}

impl PageTable {
    /// A table with every entry unused.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: [PageTableEntry::UNUSED; ENTRIES],
        }
    }

    /// Whether no entry is present.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.iter().all(|entry| !entry.is_present())
    }
}

impl Default for PageTable {
    fn default() -> Self {
        Self::new()
    }
}

const _: () = assert!(size_of::<PageTable>() as u64 == PAGE_SIZE);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_entry_sets_present_and_keeps_the_address() {
        let entry = PageTableEntry::table(PhysAddr::new(0x1234_5000), PageTableFlags::WRITABLE);
        assert_eq!(entry.bits(), 0x1234_5003);
        assert!(entry.is_present());
        assert!(!entry.is_huge());
        assert_eq!(entry.address(), PhysAddr::new(0x1234_5000));
    }

    #[test]
    fn huge_page_entry_sets_ps() {
        let entry = PageTableEntry::page(
            PhysAddr::new(0x4020_0000),
            PageSize::Size2M,
            PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE,
        );
        assert_eq!(entry.bits(), 0x8000_0000_4020_0083);
        assert!(entry.is_huge());
        assert_eq!(
            entry.page_address(PageSize::Size2M),
            PhysAddr::new(0x4020_0000)
        );
    }

    #[test]
    fn page_address_masks_the_pat_bit_of_huge_pages() {
        // Bit 12 is PAT in a 2 MiB entry (Table 5-18).
        let entry = PageTableEntry(0x0020_1083);
        assert_eq!(
            entry.page_address(PageSize::Size2M),
            PhysAddr::new(0x0020_0000)
        );
    }

    #[test]
    fn four_k_entry_does_not_set_ps() {
        let entry = PageTableEntry::page(
            PhysAddr::new(0x7000),
            PageSize::Size4K,
            PageTableFlags::EMPTY,
        );
        assert_eq!(entry.bits(), 0x7001);
    }

    #[test]
    #[should_panic(expected = "not aligned to the page size")]
    fn huge_page_entry_rejects_a_misaligned_frame() {
        let _ = PageTableEntry::page(
            PhysAddr::new(0x1000),
            PageSize::Size2M,
            PageTableFlags::EMPTY,
        );
    }

    #[test]
    fn flags_ignore_the_address_bits() {
        let entry = PageTableEntry(0xffff_f003);
        assert_eq!(
            entry.flags(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE
        );
    }

    #[test]
    fn levels_cover_the_documented_sizes() {
        assert_eq!(Level::Pml4.entry_size(), 512 << 30);
        assert_eq!(Level::Pdpt.entry_size(), PageSize::Size1G.bytes());
        assert_eq!(Level::Pd.entry_size(), PageSize::Size2M.bytes());
        assert_eq!(Level::Pt.entry_size(), PAGE_SIZE);
        assert_eq!(PageSize::Size2M.level(), Level::Pd);
    }

    #[test]
    fn flags_debug_names_each_bit() {
        let flags = PageTableFlags::PRESENT | PageTableFlags::NO_EXECUTE;
        assert_eq!(format!("{flags:?}"), "PRESENT | NO_EXECUTE");
        assert_eq!(format!("{:?}", PageTableFlags::EMPTY), "EMPTY");
    }
}
