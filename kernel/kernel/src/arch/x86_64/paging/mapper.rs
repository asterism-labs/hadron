//! Page table mapper: walks and builds x86_64 page tables via the HHDM.
//!
//! All raw pointer manipulation is delegated to [`TableWalker`] (and its
//! underlying [`HhdmAccessor`]). This file contains only the public API
//! and flag conversion logic — no direct `unsafe` pointer arithmetic.

use crate::addr::{PhysAddr, VirtAddr};
use crate::arch::x86_64::structures::paging::{PageTableEntry, PageTableFlags};
use crate::mm::mapper::{self, MapFlags, MapFlush};
use crate::paging::{Page, PhysFrame, Size1GiB, Size2MiB, Size4KiB};
use hadron_core::assert_unsafe_precondition;

use super::table::TableWalker;

/// Result of translating a virtual address.
#[derive(Debug, Clone, Copy)]
pub enum TranslateResult {
    /// Mapped via a 4 KiB page.
    Page4KiB {
        /// Physical frame.
        frame: PhysFrame<Size4KiB>,
        /// Page table entry flags.
        flags: PageTableFlags,
    },
    /// Mapped via a 2 MiB huge page.
    Page2MiB {
        /// Physical start address of the 2 MiB page.
        phys_start: PhysAddr,
        /// Page table entry flags.
        flags: PageTableFlags,
    },
    /// Mapped via a 1 GiB huge page.
    Page1GiB {
        /// Physical start address of the 1 GiB page.
        phys_start: PhysAddr,
        /// Page table entry flags.
        flags: PageTableFlags,
    },
    /// The address is not mapped.
    NotMapped,
}

/// Error type for unmap operations.
#[derive(Debug, Clone, Copy)]
pub enum UnmapError {
    /// The page is not mapped.
    NotMapped,
    /// The entry is a huge page (2 MiB or 1 GiB) and cannot be unmapped as 4 KiB.
    HugePage,
}

/// Utility for walking and building page tables via the HHDM.
///
/// All physical addresses are accessed through `hhdm_offset + phys_addr`.
/// Raw pointer manipulation is delegated to the internal [`TableWalker`].
pub struct PageTableMapper {
    walker: TableWalker,
}

impl PageTableMapper {
    /// Creates a new mapper with the given HHDM offset.
    pub fn new(hhdm_offset: VirtAddr) -> Self {
        Self {
            walker: TableWalker::new(hhdm_offset),
        }
    }

    /// Maps a 2 MiB huge page.
    ///
    /// Walks PML4 -> PDPT -> PD, allocating intermediate tables as needed.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must ensure the mapping does not conflict with existing mappings.
    pub unsafe fn map_2mib(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        phys_addr: PhysAddr,
        flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) {
        assert_unsafe_precondition!(
            phys_addr.is_aligned(0x20_0000),
            "map_2mib: physical address {:#x} is not 2 MiB aligned",
            phys_addr.as_u64()
        );
        let pd_idx = virt_addr.pd_index().as_usize();
        let intermediate = Self::intermediate_flags_for(flags);

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pd_phys = unsafe {
            self.walker
                .walk_to_pd(pml4_phys, virt_addr, intermediate, alloc)
        };
        // SAFETY: pd_phys was just ensured to be valid.
        let mut pd = unsafe { self.walker.table_at(pd_phys) };
        pd.set_entry(
            pd_idx,
            PageTableEntry::new(phys_addr, flags | PageTableFlags::HUGE_PAGE),
        );
    }

    /// Maps a 1 GiB huge page.
    ///
    /// Walks PML4 -> PDPT, allocating the intermediate PDPT table as needed.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - `phys_addr` must be 1 GiB aligned.
    /// - The caller must ensure the mapping does not conflict with existing mappings.
    pub unsafe fn map_1gib(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        phys_addr: PhysAddr,
        flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) {
        assert_unsafe_precondition!(
            phys_addr.is_aligned(0x4000_0000),
            "map_1gib: physical address {:#x} is not 1 GiB aligned",
            phys_addr.as_u64()
        );
        let pdpt_idx = virt_addr.pdpt_index().as_usize();
        let intermediate = Self::intermediate_flags_for(flags);

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pdpt_phys = unsafe {
            self.walker
                .walk_to_pdpt(pml4_phys, virt_addr, intermediate, alloc)
        };
        // SAFETY: pdpt_phys was just ensured to be valid.
        let mut pdpt = unsafe { self.walker.table_at(pdpt_phys) };
        pdpt.set_entry(
            pdpt_idx,
            PageTableEntry::new(phys_addr, flags | PageTableFlags::HUGE_PAGE),
        );
    }

    /// Maps a 4 KiB page.
    ///
    /// Walks PML4 -> PDPT -> PD -> PT, allocating intermediate tables as needed.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must ensure the mapping does not conflict with existing mappings.
    pub unsafe fn map_4k(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        phys_addr: PhysAddr,
        flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) {
        assert_unsafe_precondition!(
            phys_addr.is_aligned(4096),
            "map_4k: physical address {:#x} is not 4 KiB aligned",
            phys_addr.as_u64()
        );
        let pt_idx = virt_addr.pt_index().as_usize();
        let intermediate = Self::intermediate_flags_for(flags);

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pt_phys = unsafe {
            self.walker
                .walk_to_pt(pml4_phys, virt_addr, intermediate, alloc)
        };
        // SAFETY: pt_phys was just ensured to be valid.
        let mut pt = unsafe { self.walker.table_at(pt_phys) };
        pt.set_entry(pt_idx, PageTableEntry::new(phys_addr, flags));
    }

    /// Unmaps a 4 KiB page and returns the physical frame that was mapped.
    ///
    /// Does NOT flush the TLB -- the caller must do that.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must flush the TLB for `virt_addr` after unmapping.
    pub unsafe fn unmap_4k(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
    ) -> Result<PhysFrame<Size4KiB>, UnmapError> {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();
        let pd_idx = virt_addr.pd_index().as_usize();
        let pt_idx = virt_addr.pt_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pml4 = unsafe { self.walker.table_at(pml4_phys) };
        let pml4e = pml4.entry(pml4_idx);
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        // SAFETY: pml4e is present so its address is a valid PDPT.
        let pdpt = unsafe { self.walker.table_at(pml4e.address()) };
        let pdpte = pdpt.entry(pdpt_idx);
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 1 GiB page
        }

        // SAFETY: pdpte is present and not huge, so its address is a valid PD.
        let pd = unsafe { self.walker.table_at(pdpte.address()) };
        let pde = pd.entry(pd_idx);
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 2 MiB page
        }

        // SAFETY: pde is present and not huge, so its address is a valid PT.
        let mut pt = unsafe { self.walker.table_at(pde.address()) };
        let pte = pt.entry(pt_idx);
        if !pte.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let frame = PhysFrame::containing_address(pte.address());
        pt.clear_entry(pt_idx);
        Ok(frame)
    }

    /// Translates a virtual address, returning information about the mapping.
    ///
    /// # Safety
    /// `pml4_phys` must point to a valid PML4 table.
    pub unsafe fn translate(&self, pml4_phys: PhysAddr, virt_addr: VirtAddr) -> TranslateResult {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();
        let pd_idx = virt_addr.pd_index().as_usize();
        let pt_idx = virt_addr.pt_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pml4 = unsafe { self.walker.table_at(pml4_phys) };
        let pml4e = pml4.entry(pml4_idx);
        if !pml4e.is_present() {
            return TranslateResult::NotMapped;
        }

        // SAFETY: pml4e is present.
        let pdpt = unsafe { self.walker.table_at(pml4e.address()) };
        let pdpte = pdpt.entry(pdpt_idx);
        if !pdpte.is_present() {
            return TranslateResult::NotMapped;
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return TranslateResult::Page1GiB {
                phys_start: pdpte.address(),
                flags: pdpte.flags(),
            };
        }

        // SAFETY: pdpte is present and not huge.
        let pd = unsafe { self.walker.table_at(pdpte.address()) };
        let pde = pd.entry(pd_idx);
        if !pde.is_present() {
            return TranslateResult::NotMapped;
        }
        if pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return TranslateResult::Page2MiB {
                phys_start: pde.address(),
                flags: pde.flags(),
            };
        }

        // SAFETY: pde is present and not huge.
        let pt = unsafe { self.walker.table_at(pde.address()) };
        let pte = pt.entry(pt_idx);
        if !pte.is_present() {
            return TranslateResult::NotMapped;
        }

        TranslateResult::Page4KiB {
            frame: PhysFrame::containing_address(pte.address()),
            flags: pte.flags(),
        }
    }

    /// Translates a virtual address to a physical address, returning `None` if
    /// not mapped. Handles all page sizes.
    ///
    /// # Safety
    /// `pml4_phys` must point to a valid PML4 table.
    pub unsafe fn translate_addr(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
    ) -> Option<PhysAddr> {
        // SAFETY: Caller guarantees pml4_phys is valid.
        match unsafe { self.translate(pml4_phys, virt_addr) } {
            TranslateResult::Page4KiB { frame, .. } => {
                Some(frame.start_address() + virt_addr.page_offset())
            }
            TranslateResult::Page2MiB { phys_start, .. } => {
                let offset = virt_addr.as_u64() & 0x1F_FFFF; // 2 MiB offset
                Some(phys_start + offset)
            }
            TranslateResult::Page1GiB { phys_start, .. } => {
                let offset = virt_addr.as_u64() & 0x3FFF_FFFF; // 1 GiB offset
                Some(phys_start + offset)
            }
            TranslateResult::NotMapped => None,
        }
    }

    /// Updates the flags of a 4 KiB page mapping.
    ///
    /// Does NOT flush the TLB -- the caller must do that.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must flush the TLB for `virt_addr` after updating flags.
    pub unsafe fn update_flags_4k(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        new_flags: PageTableFlags,
    ) -> Result<(), UnmapError> {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();
        let pd_idx = virt_addr.pd_index().as_usize();
        let pt_idx = virt_addr.pt_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pml4 = unsafe { self.walker.table_at(pml4_phys) };
        let pml4e = pml4.entry(pml4_idx);
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        // SAFETY: pml4e is present.
        let pdpt = unsafe { self.walker.table_at(pml4e.address()) };
        let pdpte = pdpt.entry(pdpt_idx);
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage);
        }

        // SAFETY: pdpte is present and not huge.
        let pd = unsafe { self.walker.table_at(pdpte.address()) };
        let pde = pd.entry(pd_idx);
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage);
        }

        // SAFETY: pde is present and not huge.
        let mut pt = unsafe { self.walker.table_at(pde.address()) };
        let pte = pt.entry(pt_idx);
        if !pte.is_present() {
            return Err(UnmapError::NotMapped);
        }

        pt.set_entry(pt_idx, PageTableEntry::new(pte.address(), new_flags));
        Ok(())
    }

    /// Unmaps a 2 MiB huge page and returns the physical frame that was mapped.
    ///
    /// Does NOT flush the TLB -- the caller must do that.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must flush the TLB for `virt_addr` after unmapping.
    pub unsafe fn unmap_2mib(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
    ) -> Result<PhysFrame<Size2MiB>, UnmapError> {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();
        let pd_idx = virt_addr.pd_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pml4 = unsafe { self.walker.table_at(pml4_phys) };
        let pml4e = pml4.entry(pml4_idx);
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        // SAFETY: pml4e is present.
        let pdpt = unsafe { self.walker.table_at(pml4e.address()) };
        let pdpte = pdpt.entry(pdpt_idx);
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 1 GiB page, not 2 MiB
        }

        // SAFETY: pdpte is present and not huge.
        let mut pd = unsafe { self.walker.table_at(pdpte.address()) };
        let pde = pd.entry(pd_idx);
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 4 KiB pages, not 2 MiB
        }

        let frame = PhysFrame::containing_address(pde.address());
        pd.clear_entry(pd_idx);
        Ok(frame)
    }

    /// Unmaps a 1 GiB huge page and returns the physical frame that was mapped.
    ///
    /// Does NOT flush the TLB -- the caller must do that.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must flush the TLB for `virt_addr` after unmapping.
    pub unsafe fn unmap_1gib(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
    ) -> Result<PhysFrame<Size1GiB>, UnmapError> {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pml4 = unsafe { self.walker.table_at(pml4_phys) };
        let pml4e = pml4.entry(pml4_idx);
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        // SAFETY: pml4e is present.
        let mut pdpt = unsafe { self.walker.table_at(pml4e.address()) };
        let pdpte = pdpt.entry(pdpt_idx);
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // not a 1 GiB page
        }

        let frame = PhysFrame::containing_address(pdpte.address());
        pdpt.clear_entry(pdpt_idx);
        Ok(frame)
    }

    /// Updates the flags of a 2 MiB huge page mapping.
    ///
    /// Does NOT flush the TLB -- the caller must do that.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must flush the TLB for `virt_addr` after updating flags.
    pub unsafe fn update_flags_2mib(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        new_flags: PageTableFlags,
    ) -> Result<(), UnmapError> {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();
        let pd_idx = virt_addr.pd_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pml4 = unsafe { self.walker.table_at(pml4_phys) };
        let pml4e = pml4.entry(pml4_idx);
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        // SAFETY: pml4e is present.
        let pdpt = unsafe { self.walker.table_at(pml4e.address()) };
        let pdpte = pdpt.entry(pdpt_idx);
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 1 GiB page, not 2 MiB
        }

        // SAFETY: pdpte is present and not huge.
        let mut pd = unsafe { self.walker.table_at(pdpte.address()) };
        let pde = pd.entry(pd_idx);
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 4 KiB pages, not 2 MiB
        }

        pd.set_entry(
            pd_idx,
            PageTableEntry::new(pde.address(), new_flags | PageTableFlags::HUGE_PAGE),
        );
        Ok(())
    }

    /// Updates the flags of a 1 GiB huge page mapping.
    ///
    /// Does NOT flush the TLB -- the caller must do that.
    ///
    /// # Safety
    /// - `pml4_phys` must point to a valid PML4 table.
    /// - The caller must flush the TLB for `virt_addr` after updating flags.
    pub unsafe fn update_flags_1gib(
        &self,
        pml4_phys: PhysAddr,
        virt_addr: VirtAddr,
        new_flags: PageTableFlags,
    ) -> Result<(), UnmapError> {
        let pml4_idx = virt_addr.pml4_index().as_usize();
        let pdpt_idx = virt_addr.pdpt_index().as_usize();

        // SAFETY: Caller guarantees pml4_phys is valid.
        let pml4 = unsafe { self.walker.table_at(pml4_phys) };
        let pml4e = pml4.entry(pml4_idx);
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        // SAFETY: pml4e is present.
        let mut pdpt = unsafe { self.walker.table_at(pml4e.address()) };
        let pdpte = pdpt.entry(pdpt_idx);
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // not a 1 GiB page
        }

        pdpt.set_entry(
            pdpt_idx,
            PageTableEntry::new(pdpte.address(), new_flags | PageTableFlags::HUGE_PAGE),
        );
        Ok(())
    }

    /// Computes intermediate page table entry flags from leaf flags.
    ///
    /// Intermediate entries are always `PRESENT | WRITABLE`. If the leaf
    /// flags include `USER`, the intermediate entries also get `USER`
    /// so that ring 3 code can traverse the page table walk.
    fn intermediate_flags_for(leaf_flags: PageTableFlags) -> PageTableFlags {
        let mut flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
        if leaf_flags.contains(PageTableFlags::USER) {
            flags |= PageTableFlags::USER;
        }
        flags
    }

    /// Converts arch-independent [`MapFlags`] to x86_64 [`PageTableFlags`].
    fn map_flags_to_native(flags: MapFlags) -> PageTableFlags {
        let mut native = PageTableFlags::PRESENT;
        if flags.contains(MapFlags::WRITABLE) {
            native |= PageTableFlags::WRITABLE;
        }
        if !flags.contains(MapFlags::EXECUTABLE) {
            native |= PageTableFlags::NO_EXECUTE;
        }
        if flags.contains(MapFlags::USER) {
            native |= PageTableFlags::USER;
        }
        if flags.contains(MapFlags::GLOBAL) {
            native |= PageTableFlags::GLOBAL;
        }
        if flags.contains(MapFlags::CACHE_DISABLE) {
            native |= PageTableFlags::CACHE_DISABLE;
        }
        if flags.contains(MapFlags::WRITE_COMBINE) {
            // PAT entry 4 = WC (programmed at boot). PAT index 4 = {PAT=1, PCD=0, PWT=0}.
            // For 4 KiB pages: set PAT_4K (bit 7), clear PCD and PWT.
            // For 2 MiB pages: set PAT_HUGE (bit 12), clear PCD and PWT.
            // We set both PAT bits here; the mapper uses PAT_4K for 4 KiB PTEs
            // and PAT_HUGE for 2 MiB PD entries (they never appear in the same entry).
            native |= PageTableFlags::PAT_4K | PageTableFlags::PAT_HUGE;
            native &= !(PageTableFlags::CACHE_DISABLE | PageTableFlags::WRITE_THROUGH);
        }
        native
    }
}

// SAFETY: `PageTableMapper` correctly manipulates x86_64 4-level page tables
// via the HHDM for 4 KiB pages.
unsafe impl mapper::PageMapper<Size4KiB> for PageTableMapper {
    unsafe fn map(
        &self,
        root: PhysAddr,
        page: Page<Size4KiB>,
        frame: PhysFrame<Size4KiB>,
        flags: MapFlags,
        alloc: &mut dyn FnMut() -> PhysFrame<Size4KiB>,
    ) -> MapFlush {
        let native = Self::map_flags_to_native(flags);
        let virt = page.start_address();
        // SAFETY: Caller guarantees root is valid.
        unsafe { self.map_4k(root, virt, frame.start_address(), native, alloc) }
        MapFlush::new(virt)
    }

    unsafe fn unmap(
        &self,
        root: PhysAddr,
        page: Page<Size4KiB>,
    ) -> Result<(PhysFrame<Size4KiB>, MapFlush), mapper::UnmapError> {
        let virt = page.start_address();
        // SAFETY: Caller guarantees root is valid.
        let frame = unsafe {
            self.unmap_4k(root, virt).map_err(|e| match e {
                UnmapError::NotMapped => mapper::UnmapError::NotMapped,
                UnmapError::HugePage => mapper::UnmapError::SizeMismatch,
            })?
        };
        Ok((frame, MapFlush::new(virt)))
    }

    unsafe fn update_flags(
        &self,
        root: PhysAddr,
        page: Page<Size4KiB>,
        flags: MapFlags,
    ) -> Result<MapFlush, mapper::UnmapError> {
        let virt = page.start_address();
        let native = Self::map_flags_to_native(flags);
        // SAFETY: Caller guarantees root is valid.
        unsafe {
            self.update_flags_4k(root, virt, native)
                .map_err(|e| match e {
                    UnmapError::NotMapped => mapper::UnmapError::NotMapped,
                    UnmapError::HugePage => mapper::UnmapError::SizeMismatch,
                })?;
        }
        Ok(MapFlush::new(virt))
    }
}

// SAFETY: `PageTableMapper` correctly manipulates x86_64 2 MiB huge pages
// via the HHDM.
unsafe impl mapper::PageMapper<Size2MiB> for PageTableMapper {
    unsafe fn map(
        &self,
        root: PhysAddr,
        page: Page<Size2MiB>,
        frame: PhysFrame<Size2MiB>,
        flags: MapFlags,
        alloc: &mut dyn FnMut() -> PhysFrame<Size4KiB>,
    ) -> MapFlush {
        let native = Self::map_flags_to_native(flags);
        let virt = page.start_address();
        // SAFETY: Caller guarantees root is valid.
        unsafe { self.map_2mib(root, virt, frame.start_address(), native, alloc) }
        MapFlush::new(virt)
    }

    unsafe fn unmap(
        &self,
        root: PhysAddr,
        page: Page<Size2MiB>,
    ) -> Result<(PhysFrame<Size2MiB>, MapFlush), mapper::UnmapError> {
        let virt = page.start_address();
        // SAFETY: Caller guarantees root is valid.
        let frame = unsafe {
            self.unmap_2mib(root, virt).map_err(|e| match e {
                UnmapError::NotMapped => mapper::UnmapError::NotMapped,
                UnmapError::HugePage => mapper::UnmapError::SizeMismatch,
            })?
        };
        Ok((frame, MapFlush::new(virt)))
    }

    unsafe fn update_flags(
        &self,
        root: PhysAddr,
        page: Page<Size2MiB>,
        flags: MapFlags,
    ) -> Result<MapFlush, mapper::UnmapError> {
        let virt = page.start_address();
        let native = Self::map_flags_to_native(flags);
        // SAFETY: Caller guarantees root is valid.
        unsafe {
            self.update_flags_2mib(root, virt, native)
                .map_err(|e| match e {
                    UnmapError::NotMapped => mapper::UnmapError::NotMapped,
                    UnmapError::HugePage => mapper::UnmapError::SizeMismatch,
                })?;
        }
        Ok(MapFlush::new(virt))
    }
}

// SAFETY: `PageTableMapper` correctly manipulates x86_64 1 GiB huge pages
// via the HHDM.
unsafe impl mapper::PageMapper<Size1GiB> for PageTableMapper {
    unsafe fn map(
        &self,
        root: PhysAddr,
        page: Page<Size1GiB>,
        frame: PhysFrame<Size1GiB>,
        flags: MapFlags,
        alloc: &mut dyn FnMut() -> PhysFrame<Size4KiB>,
    ) -> MapFlush {
        let native = Self::map_flags_to_native(flags);
        let virt = page.start_address();
        // SAFETY: Caller guarantees root is valid.
        unsafe { self.map_1gib(root, virt, frame.start_address(), native, alloc) }
        MapFlush::new(virt)
    }

    unsafe fn unmap(
        &self,
        root: PhysAddr,
        page: Page<Size1GiB>,
    ) -> Result<(PhysFrame<Size1GiB>, MapFlush), mapper::UnmapError> {
        let virt = page.start_address();
        // SAFETY: Caller guarantees root is valid.
        let frame = unsafe {
            self.unmap_1gib(root, virt).map_err(|e| match e {
                UnmapError::NotMapped => mapper::UnmapError::NotMapped,
                UnmapError::HugePage => mapper::UnmapError::SizeMismatch,
            })?
        };
        Ok((frame, MapFlush::new(virt)))
    }

    unsafe fn update_flags(
        &self,
        root: PhysAddr,
        page: Page<Size1GiB>,
        flags: MapFlags,
    ) -> Result<MapFlush, mapper::UnmapError> {
        let virt = page.start_address();
        let native = Self::map_flags_to_native(flags);
        // SAFETY: Caller guarantees root is valid.
        unsafe {
            self.update_flags_1gib(root, virt, native)
                .map_err(|e| match e {
                    UnmapError::NotMapped => mapper::UnmapError::NotMapped,
                    UnmapError::HugePage => mapper::UnmapError::SizeMismatch,
                })?;
        }
        Ok(MapFlush::new(virt))
    }
}

// SAFETY: `PageTableMapper` correctly walks x86_64 4-level page tables
// for address translation via the HHDM.
unsafe impl mapper::PageTranslator for PageTableMapper {
    unsafe fn translate_addr(&self, root: PhysAddr, virt: VirtAddr) -> Option<PhysAddr> {
        // SAFETY: Caller guarantees root is valid.
        unsafe { self.translate_addr(root, virt) }
    }
}
