//! Safe page table abstractions.
//!
//! This module provides safe wrappers around the two fundamental unsafe
//! operations in page table management:
//!
//! 1. Converting a physical address to a `&mut PageTable` via the HHDM
//!    ([`HhdmAccessor::table_ref_at`])
//! 2. Zeroing a newly allocated physical frame ([`HhdmAccessor::zero_frame`])
//!
//! All other page table operations are built safely on top of these primitives
//! through [`PageTableRef`] (bounds-checked entry access) and [`TableWalker`]
//! (4-level page table traversal with automatic intermediate table allocation
//! and huge page splitting).

use crate::addr::{PhysAddr, VirtAddr};
use crate::arch::x86_64::structures::paging::{PageTable, PageTableEntry, PageTableFlags};
use crate::mm::PAGE_SIZE;
use crate::paging::{PhysFrame, Size4KiB};
use hadron_core::assert_unsafe_precondition;

// ---------------------------------------------------------------------------
// HhdmAccessor
// ---------------------------------------------------------------------------

/// Provides HHDM-based access to physical page table frames.
///
/// Encapsulates the two fundamental unsafe primitives needed for page
/// table manipulation: dereferencing a physical address as a `&mut PageTable`,
/// and zeroing a freshly allocated frame.
#[derive(Clone, Copy)]
pub struct HhdmAccessor {
    hhdm_offset: VirtAddr,
}

impl HhdmAccessor {
    /// Creates a new accessor with the given HHDM offset.
    pub fn new(hhdm_offset: VirtAddr) -> Self {
        Self { hhdm_offset }
    }

    /// Converts a physical address to its HHDM virtual address.
    fn phys_to_virt(self, phys: PhysAddr) -> *mut u8 {
        let p = phys.as_u64();
        assert!(
            p <= u64::MAX - self.hhdm_offset.as_u64(),
            "phys_to_virt: physical address {:#x} overflows HHDM (offset {:#x})",
            p,
            self.hhdm_offset.as_u64(),
        );
        (self.hhdm_offset + p).as_mut_ptr::<u8>()
    }

    /// Returns a [`PageTableRef`] wrapping the page table at the given
    /// physical address.
    ///
    /// # Safety
    ///
    /// `phys` must point to a valid, 4 KiB-aligned physical frame containing
    /// a [`PageTable`] that is accessible through the HHDM. The returned
    /// reference borrows the table for `'a`; the caller must ensure no
    /// aliasing mutable references exist.
    pub unsafe fn table_ref_at(&self, phys: PhysAddr) -> PageTableRef<'_> {
        assert_unsafe_precondition!(
            phys.is_aligned(4096),
            "table_ref_at: physical address {:#x} is not page-aligned",
            phys.as_u64()
        );
        // SAFETY: Caller guarantees phys points to a valid, aligned PageTable
        // accessible through the HHDM with no aliasing.
        let table = unsafe { &mut *(self.phys_to_virt(phys) as *mut PageTable) };
        PageTableRef { table }
    }

    /// Zeroes a 4 KiB physical frame.
    ///
    /// # Safety
    ///
    /// `phys` must point to a valid, 4 KiB-aligned physical frame that is
    /// accessible through the HHDM. The frame must not be concurrently
    /// accessed.
    pub unsafe fn zero_frame(&self, phys: PhysAddr) {
        // SAFETY: Caller guarantees the frame is valid, aligned, and exclusively
        // accessible. Dispatched via compiler builtin → alt-fn (ERMS/SSE2).
        unsafe {
            core::ptr::write_bytes(self.phys_to_virt(phys), 0, PAGE_SIZE);
        }
    }
}

// ---------------------------------------------------------------------------
// PageTableRef
// ---------------------------------------------------------------------------

/// Indicates a table walk encountered a huge page entry.
#[derive(Debug, Clone, Copy)]
pub struct HugePageEntry {
    /// The physical address stored in the huge page entry.
    pub address: PhysAddr,
    /// The flags on the huge page entry.
    pub flags: PageTableFlags,
}

/// Safe wrapper around `&mut PageTable` providing bounds-checked entry access.
///
/// All index parameters are `usize` and are bounds-checked (indices must be
/// in `0..512`).
pub struct PageTableRef<'a> {
    table: &'a mut PageTable,
}

impl<'a> PageTableRef<'a> {
    /// Reads the entry at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index >= 512`.
    pub fn entry(&self, index: usize) -> PageTableEntry {
        self.table.entries[index]
    }

    /// Writes `entry` at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index >= 512`.
    pub fn set_entry(&mut self, index: usize, entry: PageTableEntry) {
        self.table.entries[index] = entry;
    }

    /// Returns the physical address of the next-level table at `index`,
    /// or `Err(HugePageEntry)` if the entry is a huge page.
    ///
    /// Returns `Ok(None)` if the entry is not present.
    pub fn next_table_phys(&self, index: usize) -> Result<Option<PhysAddr>, HugePageEntry> {
        let entry = self.table.entries[index];
        if !entry.is_present() {
            return Ok(None);
        }
        if entry.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(HugePageEntry {
                address: entry.address(),
                flags: entry.flags(),
            });
        }
        Ok(Some(entry.address()))
    }

    /// Clears the entry at `index` (sets it to not-present).
    pub fn clear_entry(&mut self, index: usize) {
        self.table.entries[index] = PageTableEntry::empty();
    }
}

// ---------------------------------------------------------------------------
// TableWalker
// ---------------------------------------------------------------------------

/// Safe 4-level page table walker using [`HhdmAccessor`].
///
/// Provides methods for:
/// - Ensuring intermediate tables exist (allocating if needed)
/// - Splitting huge pages into finer-grained tables
/// - Walking to specific page table levels
///
/// The walker's methods are `unsafe` because they dereference physical
/// addresses through the HHDM, but they centralize all pointer manipulation
/// in [`HhdmAccessor`], so callers never touch raw pointers directly.
pub struct TableWalker {
    accessor: HhdmAccessor,
}

impl TableWalker {
    /// Creates a new walker with the given HHDM offset.
    pub fn new(hhdm_offset: VirtAddr) -> Self {
        Self {
            accessor: HhdmAccessor::new(hhdm_offset),
        }
    }

    /// Returns the underlying [`HhdmAccessor`].
    pub fn accessor(&self) -> &HhdmAccessor {
        &self.accessor
    }

    /// Ensures `table[index]` points to a valid next-level table, allocating
    /// one if not present. If the entry is a huge page, it is split.
    ///
    /// Returns the physical address of the next-level table.
    ///
    /// # Safety
    ///
    /// `table_phys` must point to a valid, HHDM-accessible page table.
    pub unsafe fn ensure_table(
        &self,
        table_phys: PhysAddr,
        index: usize,
        intermediate_flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) -> PhysAddr {
        // SAFETY: Caller guarantees table_phys is valid.
        let mut table = unsafe { self.accessor.table_ref_at(table_phys) };
        let entry = table.entry(index);

        if entry.is_present() {
            if entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                // SAFETY: table_phys is valid and entry is a present huge page.
                return unsafe {
                    self.split_huge_page(table_phys, index, entry, intermediate_flags, alloc)
                };
            }
            // OR in any new flags (e.g. USER for mixed kernel/user subtrees).
            let combined = entry.flags() | intermediate_flags;
            if combined != entry.flags() {
                table.set_entry(index, PageTableEntry::new(entry.address(), combined));
            }
            entry.address()
        } else {
            let new_frame = alloc().start_address();
            // SAFETY: Frame was just allocated and is HHDM-accessible.
            unsafe { self.accessor.zero_frame(new_frame) };
            table.set_entry(index, PageTableEntry::new(new_frame, intermediate_flags));
            new_frame
        }
    }

    /// Splits a huge page entry into a next-level table with 512 entries
    /// preserving the existing mapping at finer granularity.
    ///
    /// - 1 GiB PDPT entry → 512 × 2 MiB PD entries (each still HUGE_PAGE)
    /// - 2 MiB PD entry → 512 × 4 KiB PT entries
    ///
    /// # Safety
    ///
    /// `table_phys` must be valid and `table[index]` must be a present huge
    /// page entry.
    unsafe fn split_huge_page(
        &self,
        table_phys: PhysAddr,
        index: usize,
        entry: PageTableEntry,
        intermediate_flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) -> PhysAddr {
        let huge_phys = entry.address();
        let orig_flags = entry.flags();
        let sub_flags = PageTableFlags::from_bits_truncate(
            orig_flags.bits() & !PageTableFlags::HUGE_PAGE.bits(),
        );

        let new_frame = alloc().start_address();
        // SAFETY: Frame was just allocated and is HHDM-accessible.
        unsafe { self.accessor.zero_frame(new_frame) };

        // SAFETY: new_frame was just allocated and zeroed.
        let mut new_table = unsafe { self.accessor.table_ref_at(new_frame) };

        // 1 GiB pages are 0x4000_0000-aligned, 2 MiB pages are 0x20_0000-aligned.
        let is_1gib = huge_phys.is_aligned(0x4000_0000);
        if is_1gib {
            // 1 GiB → 512 × 2 MiB (sub-entries are still huge pages).
            let stride = 0x20_0000_u64;
            for i in 0..512 {
                // SAFETY: Each sub-address is within the original 1 GiB range.
                let sub_phys = unsafe { PhysAddr::new_unchecked(huge_phys.as_u64() + i * stride) };
                new_table.set_entry(
                    i as usize,
                    PageTableEntry::new(sub_phys, sub_flags | PageTableFlags::HUGE_PAGE),
                );
            }
        } else {
            // 2 MiB → 512 × 4 KiB (sub-entries are regular pages).
            let stride = 4096_u64;
            for i in 0..512 {
                // SAFETY: Each sub-address is within the original 2 MiB range.
                let sub_phys = unsafe { PhysAddr::new_unchecked(huge_phys.as_u64() + i * stride) };
                new_table.set_entry(i as usize, PageTableEntry::new(sub_phys, sub_flags));
            }
        }

        // Replace the huge page entry with a pointer to the new table.
        // SAFETY: table_phys is valid.
        let mut table = unsafe { self.accessor.table_ref_at(table_phys) };
        table.set_entry(index, PageTableEntry::new(new_frame, intermediate_flags));
        new_frame
    }

    /// Walks from PML4 through to the PT level for a 4 KiB mapping,
    /// allocating intermediate tables as needed.
    ///
    /// Returns the physical address of the PT containing the final entry.
    ///
    /// # Safety
    ///
    /// `pml4_phys` must point to a valid PML4 table.
    pub unsafe fn walk_to_pt(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        intermediate_flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) -> PhysAddr {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();
        let pd_idx = virt_addr.pd_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pdpt_phys =
            unsafe { self.ensure_table(pml4_phys, pml4_idx, intermediate_flags, alloc) };
        let pd_phys = unsafe { self.ensure_table(pdpt_phys, pdpt_idx, intermediate_flags, alloc) };
        unsafe { self.ensure_table(pd_phys, pd_idx, intermediate_flags, alloc) }
    }

    /// Walks from PML4 through to the PD level for a 2 MiB mapping,
    /// allocating intermediate tables as needed.
    ///
    /// Returns the physical address of the PD containing the final entry.
    ///
    /// # Safety
    ///
    /// `pml4_phys` must point to a valid PML4 table.
    pub unsafe fn walk_to_pd(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        intermediate_flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) -> PhysAddr {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pdpt_phys =
            unsafe { self.ensure_table(pml4_phys, pml4_idx, intermediate_flags, alloc) };
        unsafe { self.ensure_table(pdpt_phys, pdpt_idx, intermediate_flags, alloc) }
    }

    /// Walks from PML4 through to the PDPT level for a 1 GiB mapping,
    /// allocating the PDPT table if needed.
    ///
    /// Returns the physical address of the PDPT containing the final entry.
    ///
    /// # Safety
    ///
    /// `pml4_phys` must point to a valid PML4 table.
    pub unsafe fn walk_to_pdpt(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        intermediate_flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) -> PhysAddr {
        let pml4_idx = virt_addr.pml4_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        unsafe { self.ensure_table(pml4_phys, pml4_idx, intermediate_flags, alloc) }
    }

    /// Read-only walk returning a [`PageTableRef`] at the given physical
    /// address. Convenience wrapper around [`HhdmAccessor::table_ref_at`].
    ///
    /// # Safety
    ///
    /// `phys` must point to a valid, HHDM-accessible page table.
    pub unsafe fn table_at(&self, phys: PhysAddr) -> PageTableRef<'_> {
        // SAFETY: Caller guarantees phys is valid.
        unsafe { self.accessor.table_ref_at(phys) }
    }
}
