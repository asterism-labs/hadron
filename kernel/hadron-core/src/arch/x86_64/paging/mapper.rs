//! Page table mapper: walks and builds x86_64 page tables via the HHDM.

use crate::addr::{PhysAddr, VirtAddr};
use crate::arch::x86_64::structures::paging::{PageTable, PageTableEntry, PageTableFlags};
use crate::mm::PAGE_SIZE;
use crate::mm::mapper::{self, MapFlags, MapFlush};
use crate::paging::{Page, PhysFrame, Size1GiB, Size2MiB, Size4KiB};

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
pub struct PageTableMapper {
    hhdm_offset: u64,
}

impl PageTableMapper {
    /// Creates a new mapper with the given HHDM offset.
    pub fn new(hhdm_offset: u64) -> Self {
        Self { hhdm_offset }
    }

    /// Converts a physical address to its HHDM virtual address.
    fn phys_to_virt(&self, phys: PhysAddr) -> *mut u8 {
        let p = phys.as_u64();
        assert!(
            p <= u64::MAX - self.hhdm_offset,
            "phys_to_virt: physical address {:#x} overflows HHDM (offset {:#x})",
            p,
            self.hhdm_offset,
        );
        (self.hhdm_offset + p) as *mut u8
    }

    /// Returns a mutable reference to the [`PageTable`] at `phys`.
    ///
    /// # Safety
    /// `phys` must point to a valid, 4 KiB-aligned physical frame that is
    /// accessible through the HHDM.
    unsafe fn table_at(&self, phys: PhysAddr) -> &mut PageTable {
        unsafe { &mut *(self.phys_to_virt(phys) as *mut PageTable) }
    }

    /// Ensures the entry at `table[index]` points to a valid next-level table,
    /// allocating one if it is not present. Returns the physical address of the
    /// next-level table.
    ///
    /// `intermediate_flags` are applied to the entry (e.g. `PRESENT | WRITABLE`,
    /// with `USER` added for user-accessible mappings). If the entry already
    /// exists, any missing flags from `intermediate_flags` are OR'd in.
    ///
    /// Newly allocated frames are zeroed before use so that no stale data is
    /// misinterpreted as present page table entries.
    ///
    /// # Safety
    /// The caller must ensure `table_phys` is valid and accessible through the HHDM.
    unsafe fn ensure_table(
        &self,
        table_phys: PhysAddr,
        index: usize,
        intermediate_flags: PageTableFlags,
        alloc: &mut (impl FnMut() -> PhysFrame<Size4KiB> + ?Sized),
    ) -> PhysAddr {
        let table = unsafe { self.table_at(table_phys) };
        let entry = table.entries[index];
        if entry.is_present() {
            // OR in any new flags (e.g. USER for mixed kernel/user subtrees).
            let combined = entry.flags() | intermediate_flags;
            if combined != entry.flags() {
                table.entries[index] = PageTableEntry::new(entry.address(), combined);
            }
            entry.address()
        } else {
            let new_frame = alloc().start_address();
            // SAFETY: The frame was just allocated and is accessible through the HHDM.
            // Zeroing ensures no stale PTEs are misinterpreted as present entries.
            unsafe {
                core::ptr::write_bytes(self.phys_to_virt(new_frame), 0, PAGE_SIZE);
            }
            table.entries[index] = PageTableEntry::new(new_frame, intermediate_flags);
            new_frame
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();
        let pd_idx = virt_addr.pd_index();

        // Derive intermediate flags: PRESENT | WRITABLE, plus USER if leaf has USER.
        let intermediate = Self::intermediate_flags_for(flags);
        let pdpt_phys = unsafe { self.ensure_table(pml4_phys, pml4_idx, intermediate, alloc) };
        let pd_phys = unsafe { self.ensure_table(pdpt_phys, pdpt_idx, intermediate, alloc) };

        let pd = unsafe { self.table_at(pd_phys) };
        pd.entries[pd_idx] = PageTableEntry::new(phys_addr, flags | PageTableFlags::HUGE_PAGE);
    }

    /// Maps a 1 GiB huge page.
    ///
    /// Walks PML4 -> PDPT, allocating the intermediate PDPT table as needed.
    /// Writes a 1 GiB huge page entry directly into the PDPT.
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();

        let intermediate = Self::intermediate_flags_for(flags);
        // SAFETY: Caller guarantees pml4_phys is valid.
        let pdpt_phys = unsafe { self.ensure_table(pml4_phys, pml4_idx, intermediate, alloc) };

        // SAFETY: pdpt_phys was just ensured to be a valid PDPT table.
        let pdpt = unsafe { self.table_at(pdpt_phys) };
        pdpt.entries[pdpt_idx] = PageTableEntry::new(phys_addr, flags | PageTableFlags::HUGE_PAGE);
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();
        let pd_idx = virt_addr.pd_index();
        let pt_idx = virt_addr.pt_index();

        let intermediate = Self::intermediate_flags_for(flags);
        let pdpt_phys = unsafe { self.ensure_table(pml4_phys, pml4_idx, intermediate, alloc) };
        let pd_phys = unsafe { self.ensure_table(pdpt_phys, pdpt_idx, intermediate, alloc) };
        let pt_phys = unsafe { self.ensure_table(pd_phys, pd_idx, intermediate, alloc) };

        let pt = unsafe { self.table_at(pt_phys) };
        pt.entries[pt_idx] = PageTableEntry::new(phys_addr, flags);
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();
        let pd_idx = virt_addr.pd_index();
        let pt_idx = virt_addr.pt_index();

        let pml4 = unsafe { self.table_at(pml4_phys) };
        let pml4e = pml4.entries[pml4_idx];
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let pdpt = unsafe { self.table_at(pml4e.address()) };
        let pdpte = pdpt.entries[pdpt_idx];
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 1 GiB page
        }

        let pd = unsafe { self.table_at(pdpte.address()) };
        let pde = pd.entries[pd_idx];
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 2 MiB page
        }

        let pt = unsafe { self.table_at(pde.address()) };
        let pte = pt.entries[pt_idx];
        if !pte.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let frame = PhysFrame::containing_address(pte.address());
        pt.entries[pt_idx] = PageTableEntry::empty();
        Ok(frame)
    }

    /// Translates a virtual address, returning information about the mapping.
    ///
    /// # Safety
    /// `pml4_phys` must point to a valid PML4 table.
    pub unsafe fn translate(&self, pml4_phys: PhysAddr, virt_addr: VirtAddr) -> TranslateResult {
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();
        let pd_idx = virt_addr.pd_index();
        let pt_idx = virt_addr.pt_index();

        let pml4 = unsafe { self.table_at(pml4_phys) };
        let pml4e = pml4.entries[pml4_idx];
        if !pml4e.is_present() {
            return TranslateResult::NotMapped;
        }

        let pdpt = unsafe { self.table_at(pml4e.address()) };
        let pdpte = pdpt.entries[pdpt_idx];
        if !pdpte.is_present() {
            return TranslateResult::NotMapped;
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return TranslateResult::Page1GiB {
                phys_start: pdpte.address(),
                flags: pdpte.flags(),
            };
        }

        let pd = unsafe { self.table_at(pdpte.address()) };
        let pde = pd.entries[pd_idx];
        if !pde.is_present() {
            return TranslateResult::NotMapped;
        }
        if pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return TranslateResult::Page2MiB {
                phys_start: pde.address(),
                flags: pde.flags(),
            };
        }

        let pt = unsafe { self.table_at(pde.address()) };
        let pte = pt.entries[pt_idx];
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();
        let pd_idx = virt_addr.pd_index();
        let pt_idx = virt_addr.pt_index();

        let pml4 = unsafe { self.table_at(pml4_phys) };
        let pml4e = pml4.entries[pml4_idx];
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let pdpt = unsafe { self.table_at(pml4e.address()) };
        let pdpte = pdpt.entries[pdpt_idx];
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage);
        }

        let pd = unsafe { self.table_at(pdpte.address()) };
        let pde = pd.entries[pd_idx];
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage);
        }

        let pt = unsafe { self.table_at(pde.address()) };
        let pte = pt.entries[pt_idx];
        if !pte.is_present() {
            return Err(UnmapError::NotMapped);
        }

        pt.entries[pt_idx] = PageTableEntry::new(pte.address(), new_flags);
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();
        let pd_idx = virt_addr.pd_index();

        let pml4 = unsafe { self.table_at(pml4_phys) };
        let pml4e = pml4.entries[pml4_idx];
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let pdpt = unsafe { self.table_at(pml4e.address()) };
        let pdpte = pdpt.entries[pdpt_idx];
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 1 GiB page, not 2 MiB
        }

        let pd = unsafe { self.table_at(pdpte.address()) };
        let pde = pd.entries[pd_idx];
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 4 KiB pages, not 2 MiB
        }

        let frame = PhysFrame::containing_address(pde.address());
        pd.entries[pd_idx] = PageTableEntry::empty();
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();

        let pml4 = unsafe { self.table_at(pml4_phys) };
        let pml4e = pml4.entries[pml4_idx];
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let pdpt = unsafe { self.table_at(pml4e.address()) };
        let pdpte = pdpt.entries[pdpt_idx];
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // not a 1 GiB page
        }

        let frame = PhysFrame::containing_address(pdpte.address());
        pdpt.entries[pdpt_idx] = PageTableEntry::empty();
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();
        let pd_idx = virt_addr.pd_index();

        let pml4 = unsafe { self.table_at(pml4_phys) };
        let pml4e = pml4.entries[pml4_idx];
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let pdpt = unsafe { self.table_at(pml4e.address()) };
        let pdpte = pdpt.entries[pdpt_idx];
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 1 GiB page, not 2 MiB
        }

        let pd = unsafe { self.table_at(pdpte.address()) };
        let pde = pd.entries[pd_idx];
        if !pde.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pde.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // 4 KiB pages, not 2 MiB
        }

        pd.entries[pd_idx] =
            PageTableEntry::new(pde.address(), new_flags | PageTableFlags::HUGE_PAGE);
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
        let pml4_idx = virt_addr.pml4_index();
        let pdpt_idx = virt_addr.pdpt_index();

        let pml4 = unsafe { self.table_at(pml4_phys) };
        let pml4e = pml4.entries[pml4_idx];
        if !pml4e.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let pdpt = unsafe { self.table_at(pml4e.address()) };
        let pdpte = pdpt.entries[pdpt_idx];
        if !pdpte.is_present() {
            return Err(UnmapError::NotMapped);
        }
        if !pdpte.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(UnmapError::HugePage); // not a 1 GiB page
        }

        pdpt.entries[pdpt_idx] =
            PageTableEntry::new(pdpte.address(), new_flags | PageTableFlags::HUGE_PAGE);
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
