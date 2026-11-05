//! Builds and walks `x86_64` 4-level page tables.
//!
//! The mapper owns no memory. It holds the physical address of a PML4 and
//! reaches every table through a [`DirectMap`]. New tables come from a
//! [`FrameAllocator`] and are zeroed before use.
//!
//! Tables the mapper creates are present and writable, and user-accessible
//! when a user page is mapped below them. The leaf entry alone sets the
//! page's permissions. The mapper does not flush the TLB. After changing
//! the live tables, the caller runs `invlpg` or reloads CR3.

use core::ops::Range;

use arch_x86_64::paging::{Level, PageSize, PageTable, PageTableEntry, PageTableFlags};
use arch_x86_64::{PhysAddr, VirtAddr};

use crate::direct_map::DirectMap;
use crate::frame::{FrameAllocator, PhysFrame};

/// Why a mapping failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    /// The virtual or physical address is not aligned to the page size.
    Misaligned,
    /// The page, or part of it, is already mapped.
    AlreadyMapped,
    /// A huge page covers the range, so the table below it does not exist.
    HugePageInTheWay,
    /// The frame allocator had no frame for a new table.
    OutOfFrames,
}

/// Why an unmap failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnmapError {
    /// Nothing is mapped at the address.
    NotMapped,
    /// The address is inside a mapped page but not at its start.
    NotPageStart,
}

/// Where a virtual address leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Translation {
    /// The physical address the virtual address translates to.
    pub phys: PhysAddr,
    /// The size of the page that holds it.
    pub size: PageSize,
    /// The flags of the leaf entry.
    pub flags: PageTableFlags,
}

/// A page table tree rooted at a PML4.
#[derive(Debug)]
pub struct Mapper {
    root: PhysFrame,
    direct: DirectMap,
}

impl Mapper {
    /// A mapper for the tree rooted at `root`.
    ///
    /// # Safety
    ///
    /// Every table in the tree must be reachable through `direct`, and
    /// nothing else may change the tree while the mapper exists.
    #[must_use]
    pub const unsafe fn new(root: PhysFrame, direct: DirectMap) -> Self {
        Self { root, direct }
    }

    /// A mapper for a new, empty tree whose root comes from `frames`.
    ///
    /// # Safety
    ///
    /// Frames from `frames` must be reachable through `direct` and unused.
    ///
    /// # Errors
    ///
    /// Fails with [`MapError::OutOfFrames`] if `frames` has none left.
    pub unsafe fn empty(
        direct: DirectMap,
        frames: &mut impl FrameAllocator,
    ) -> Result<Self, MapError> {
        let root = Self::new_table(direct, frames)?;
        // SAFETY: the tree is a single zeroed table that only this mapper
        // knows about.
        Ok(unsafe { Self::new(root, direct) })
    }

    /// The PML4, which goes in CR3.
    #[must_use]
    pub const fn root(&self) -> PhysFrame {
        self.root
    }

    /// The direct map used to reach the tables.
    #[must_use]
    pub const fn direct_map(&self) -> DirectMap {
        self.direct
    }

    /// Points the mapper at the tables through a different direct map.
    ///
    /// # Safety
    ///
    /// Every table in the tree must be reachable through `direct`.
    pub const unsafe fn set_direct_map(&mut self, direct: DirectMap) {
        self.direct = direct;
    }

    /// Maps the page of `size` at `page` to `frame`.
    ///
    /// # Errors
    ///
    /// Fails if either address is misaligned, the range is already
    /// mapped, a huge page covers it, or no frame is left for a table.
    pub fn map(
        &mut self,
        page: VirtAddr,
        frame: PhysAddr,
        size: PageSize,
        flags: PageTableFlags,
        frames: &mut impl FrameAllocator,
    ) -> Result<(), MapError> {
        if !page.is_aligned(size.bytes()) || !frame.is_aligned(size.bytes()) {
            return Err(MapError::Misaligned);
        }
        let mut parent_flags = PageTableFlags::WRITABLE;
        if flags.contains(PageTableFlags::USER) {
            parent_flags |= PageTableFlags::USER;
        }
        let mut table = self.root.start();
        for level in [Level::Pml4, Level::Pdpt, Level::Pd] {
            if level == size.level() {
                break;
            }
            table = self.next_table(table, page.table_index(level), parent_flags, frames)?;
        }
        let slot = self.entry(table, page.table_index(size.level()));
        // SAFETY: `slot` points into a table of this tree, and the mapper
        // has the only access.
        let entry = unsafe { slot.read() };
        if entry.is_present() {
            return Err(MapError::AlreadyMapped);
        }
        // SAFETY: as above.
        unsafe { slot.write(PageTableEntry::page(frame, size, flags)) };
        Ok(())
    }

    /// Maps `len` bytes at `virt` to `phys`, with the largest pages up to
    /// `largest` that fit. Both addresses and `len` must be 4 KiB aligned.
    ///
    /// On failure, the pages mapped before the failing one stay mapped.
    ///
    /// # Errors
    ///
    /// Fails as [`map`](Self::map) does.
    pub fn map_range(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        len: u64,
        largest: PageSize,
        flags: PageTableFlags,
        frames: &mut impl FrameAllocator,
    ) -> Result<(), MapError> {
        const SIZES: [PageSize; 3] = [PageSize::Size1G, PageSize::Size2M, PageSize::Size4K];
        let small = PageSize::Size4K.bytes();
        if !virt.is_aligned(small) || !phys.is_aligned(small) || len % small != 0 {
            return Err(MapError::Misaligned);
        }
        let mut done = 0;
        while done < len {
            let (v, p) = (virt + done, phys + done);
            let size = SIZES
                .into_iter()
                .filter(|&size| size <= largest)
                .find(|size| {
                    let bytes = size.bytes();
                    v.is_aligned(bytes) && p.is_aligned(bytes) && len - done >= bytes
                })
                .unwrap_or(PageSize::Size4K);
            self.map(v, p, size, flags, frames)?;
            done += size.bytes();
        }
        Ok(())
    }

    /// Removes the page that starts at `page` and returns what it mapped.
    /// Tables left empty stay in place.
    ///
    /// # Errors
    ///
    /// Fails if nothing is mapped there or `page` is not the start of the
    /// mapped page.
    pub fn unmap(&mut self, page: VirtAddr) -> Result<Translation, UnmapError> {
        let (slot, size) = self.find_leaf(page).ok_or(UnmapError::NotMapped)?;
        if !page.is_aligned(size.bytes()) {
            return Err(UnmapError::NotPageStart);
        }
        // SAFETY: `slot` points into a table of this tree, and the mapper
        // has the only access.
        let entry = unsafe { slot.read() };
        // SAFETY: as above.
        unsafe { slot.write(PageTableEntry::UNUSED) };
        Ok(Translation {
            phys: entry.page_address(size),
            size,
            flags: entry.flags(),
        })
    }

    /// Clears the PML4 entries in `slots` and passes every table below
    /// them to `free`, lower levels first. The pages those tables map are
    /// not freed. Returns the number of tables freed.
    ///
    /// The CPU may still hold the cleared entries in its TLB and
    /// paging-structure caches. Flush them before the freed frames are
    /// reused.
    ///
    /// # Panics
    ///
    /// Panics if `slots` reaches past entry 511.
    pub fn unmap_pml4_slots(
        &mut self,
        slots: Range<usize>,
        mut free: impl FnMut(PhysFrame),
    ) -> u64 {
        assert!(slots.end <= 512, "a PML4 has 512 entries");
        let mut freed = 0;
        for index in slots {
            let slot = self.entry(self.root.start(), index);
            // SAFETY: `slot` points into the PML4, and the mapper has the
            // only access.
            let entry = unsafe { slot.read() };
            if !entry.is_present() {
                continue;
            }
            // SAFETY: as above.
            unsafe { slot.write(PageTableEntry::UNUSED) };
            freed += self.free_table(entry.address(), Level::Pdpt, &mut free);
        }
        freed
    }

    /// Passes the table at `table`, which sits at `level`, and every table
    /// below it to `free`. Returns the number of tables.
    fn free_table(&self, table: PhysAddr, level: Level, free: &mut impl FnMut(PhysFrame)) -> u64 {
        let mut freed = 1;
        if let Some(next) = level.next() {
            for index in 0..512 {
                // SAFETY: the entry is inside a table of this tree.
                let entry = unsafe { self.entry(table, index).read() };
                if entry.is_present() && !entry.is_huge() {
                    freed += self.free_table(entry.address(), next, free);
                }
            }
        }
        free(PhysFrame::containing(table));
        freed
    }

    /// Where `virt` leads, or `None` if it is not mapped.
    #[must_use]
    pub fn translate(&self, virt: VirtAddr) -> Option<Translation> {
        let (slot, size) = self.find_leaf(virt)?;
        // SAFETY: `slot` points into a table of this tree.
        let entry = unsafe { slot.read() };
        let offset = virt.as_u64() & (size.bytes() - 1);
        Some(Translation {
            phys: entry.page_address(size) + offset,
            size,
            flags: entry.flags(),
        })
    }

    /// The present leaf entry for `virt` and the page size it maps.
    fn find_leaf(&self, virt: VirtAddr) -> Option<(*mut PageTableEntry, PageSize)> {
        let mut table = self.root.start();
        let mut level = Level::Pml4;
        loop {
            let slot = self.entry(table, virt.table_index(level));
            // SAFETY: `slot` points into a table of this tree.
            let entry = unsafe { slot.read() };
            if !entry.is_present() {
                return None;
            }
            match (level, level.page_size()) {
                (Level::Pt, Some(size)) => return Some((slot, size)),
                (Level::Pdpt | Level::Pd, Some(size)) if entry.is_huge() => {
                    return Some((slot, size));
                }
                _ => {}
            }
            table = entry.address();
            level = level.next()?;
        }
    }

    /// The table that entry `index` of `table` points at, created if it is
    /// missing.
    fn next_table(
        &mut self,
        table: PhysAddr,
        index: usize,
        flags: PageTableFlags,
        frames: &mut impl FrameAllocator,
    ) -> Result<PhysAddr, MapError> {
        let slot = self.entry(table, index);
        // SAFETY: `slot` points into a table of this tree, and the mapper
        // has the only access.
        let entry = unsafe { slot.read() };
        if !entry.is_present() {
            let next = Self::new_table(self.direct, frames)?;
            // SAFETY: as above.
            unsafe { slot.write(PageTableEntry::table(next.start(), flags)) };
            return Ok(next.start());
        }
        if entry.is_huge() {
            return Err(MapError::HugePageInTheWay);
        }
        if !entry.flags().contains(flags) {
            let widened = PageTableEntry::table(entry.address(), entry.flags() | flags);
            // SAFETY: as above.
            unsafe { slot.write(widened) };
        }
        Ok(entry.address())
    }

    /// A zeroed frame for a new table.
    fn new_table(
        direct: DirectMap,
        frames: &mut impl FrameAllocator,
    ) -> Result<PhysFrame, MapError> {
        let frame = frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
        // SAFETY: the allocator hands out unused frames, and the caller of
        // `new` or `empty` guarantees they are reachable through `direct`.
        unsafe { direct.table(frame.start()).write(PageTable::new()) };
        Ok(frame)
    }

    /// A pointer to entry `index` of the table at `table`.
    fn entry(&self, table: PhysAddr, index: usize) -> *mut PageTableEntry {
        let table = self.direct.table(table);
        // SAFETY: `index` is below 512, so the entry is inside the table.
        unsafe {
            (&raw mut (*table).entries)
                .cast::<PageTableEntry>()
                .add(index)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeMemory;

    const RW_NX: PageTableFlags = PageTableFlags::WRITABLE.union(PageTableFlags::NO_EXECUTE);

    fn kernel(addr: u64) -> VirtAddr {
        VirtAddr::new(addr)
    }

    #[test]
    fn maps_and_translates_a_4k_page() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        let page = kernel(0xffff_ffff_8000_0000);
        mapper
            .map(
                page,
                PhysAddr::new(0x20_0000),
                PageSize::Size4K,
                RW_NX,
                &mut memory,
            )
            .unwrap();
        let t = mapper.translate(page + 0x123).unwrap();
        assert_eq!(t.phys, PhysAddr::new(0x20_0123));
        assert_eq!(t.size, PageSize::Size4K);
        assert_eq!(t.flags, PageTableFlags::PRESENT | RW_NX);
        // Root, PDPT, PD and PT.
        assert_eq!(memory.tables_used(), 4);
    }

    #[test]
    fn unmapped_addresses_do_not_translate() {
        let mut memory = FakeMemory::new(16);
        let mapper = memory.mapper();
        assert_eq!(mapper.translate(VirtAddr::ZERO), None);
    }

    #[test]
    fn huge_pages_skip_lower_tables() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        let page = kernel(0xffff_8880_0000_0000);
        mapper
            .map(
                page,
                PhysAddr::new(0x4000_0000),
                PageSize::Size1G,
                RW_NX,
                &mut memory,
            )
            .unwrap();
        mapper
            .map(
                page + (1 << 30),
                PhysAddr::new(0x20_0000),
                PageSize::Size2M,
                RW_NX,
                &mut memory,
            )
            .unwrap();
        let t = mapper.translate(page + 0x1234_5678).unwrap();
        assert_eq!(t.phys, PhysAddr::new(0x5234_5678));
        assert_eq!(t.size, PageSize::Size1G);
        assert!(t.flags.contains(PageTableFlags::HUGE_PAGE));
        let t = mapper.translate(page + (1 << 30) + 0x1000).unwrap();
        assert_eq!(t.phys, PhysAddr::new(0x20_1000));
        assert_eq!(t.size, PageSize::Size2M);
        // Root, PDPT, and one PD for the 2 MiB page.
        assert_eq!(memory.tables_used(), 3);
    }

    #[test]
    fn rejects_double_mapping_and_huge_page_conflicts() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        let page = kernel(0xffff_c900_0000_0000);
        let frame = PhysAddr::new(0x20_0000);
        mapper
            .map(page, frame, PageSize::Size2M, RW_NX, &mut memory)
            .unwrap();
        assert_eq!(
            mapper.map(page, frame, PageSize::Size2M, RW_NX, &mut memory),
            Err(MapError::AlreadyMapped)
        );
        assert_eq!(
            mapper.map(page + 0x1000, frame, PageSize::Size4K, RW_NX, &mut memory),
            Err(MapError::HugePageInTheWay)
        );
    }

    #[test]
    fn rejects_misaligned_pages() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        assert_eq!(
            mapper.map(
                kernel(0xffff_c900_0000_1000),
                PhysAddr::new(0x20_0000),
                PageSize::Size2M,
                RW_NX,
                &mut memory
            ),
            Err(MapError::Misaligned)
        );
    }

    #[test]
    fn map_range_uses_the_largest_page_that_fits() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        // 4 KiB short of 2 MiB below a 2 MiB boundary, then 2 MiB, then 4 KiB.
        let virt = kernel(0xffff_8880_001f_f000);
        let phys = PhysAddr::new(0x001f_f000);
        let len = 0x1000 + 0x20_0000 + 0x1000;
        mapper
            .map_range(virt, phys, len, PageSize::Size2M, RW_NX, &mut memory)
            .unwrap();
        let sizes: Vec<_> = [0, 0x1000, 0x20_1000]
            .iter()
            .map(|&off| mapper.translate(virt + off).unwrap().size)
            .collect();
        assert_eq!(
            sizes,
            [PageSize::Size4K, PageSize::Size2M, PageSize::Size4K]
        );
        for off in (0..len).step_by(0x1000) {
            assert_eq!(mapper.translate(virt + off).unwrap().phys, phys + off);
        }
        assert_eq!(mapper.translate(virt + len), None);
    }

    #[test]
    fn map_range_respects_the_largest_size() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        let virt = kernel(0xffff_8880_0000_0000);
        mapper
            .map_range(
                virt,
                PhysAddr::ZERO,
                0x40_0000,
                PageSize::Size4K,
                RW_NX,
                &mut memory,
            )
            .unwrap();
        assert_eq!(mapper.translate(virt).unwrap().size, PageSize::Size4K);
    }

    #[test]
    fn unmap_returns_the_mapping_and_clears_it() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        let page = kernel(0xffff_ffff_8000_0000);
        let frame = PhysAddr::new(0x7000);
        mapper
            .map(page, frame, PageSize::Size4K, RW_NX, &mut memory)
            .unwrap();
        assert_eq!(mapper.unmap(page + 0x10), Err(UnmapError::NotPageStart));
        let t = mapper.unmap(page).unwrap();
        assert_eq!((t.phys, t.size), (frame, PageSize::Size4K));
        assert_eq!(mapper.translate(page), None);
        assert_eq!(mapper.unmap(page), Err(UnmapError::NotMapped));
    }

    #[test]
    fn user_pages_make_their_tables_user_accessible() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        let kernel_page = kernel(0x0000_4000_0000_0000);
        let user_page = kernel_page + 0x1000;
        let frame = PhysAddr::new(0x7000);
        mapper
            .map(kernel_page, frame, PageSize::Size4K, RW_NX, &mut memory)
            .unwrap();
        mapper
            .map(
                user_page,
                frame,
                PageSize::Size4K,
                PageTableFlags::USER,
                &mut memory,
            )
            .unwrap();
        let root = memory.table(mapper.root().start());
        let pml4e = root.entries[user_page.table_index(Level::Pml4)];
        assert!(
            pml4e
                .flags()
                .contains(PageTableFlags::USER | PageTableFlags::WRITABLE)
        );
        // The kernel page's leaf entry stays supervisor-only.
        let t = mapper.translate(kernel_page).unwrap();
        assert!(!t.flags.contains(PageTableFlags::USER));
    }

    #[test]
    fn running_out_of_frames_fails_cleanly() {
        let mut memory = FakeMemory::new(2);
        let mut mapper = memory.mapper();
        assert_eq!(
            mapper.map(
                kernel(0xffff_ffff_8000_0000),
                PhysAddr::ZERO,
                PageSize::Size4K,
                RW_NX,
                &mut memory
            ),
            Err(MapError::OutOfFrames)
        );
    }
    #[test]
    fn unmap_pml4_slots_frees_every_table_below() {
        let mut memory = FakeMemory::new(16);
        let mut mapper = memory.mapper();
        // Slot 256: a 4 KiB page and a 2 MiB page. Slot 257: a 1 GiB page.
        // Slot 300 stays mapped.
        let pages = [
            (0xffff_8000_0000_0000, PageSize::Size4K),
            (0xffff_8000_4000_0000, PageSize::Size2M),
            (0xffff_8080_0000_0000, PageSize::Size1G),
            (0xffff_9600_0000_0000, PageSize::Size4K),
        ];
        for (virt, size) in pages {
            mapper
                .map(kernel(virt), PhysAddr::ZERO, size, RW_NX, &mut memory)
                .unwrap();
        }
        let mut freed = Vec::new();
        let count = mapper.unmap_pml4_slots(256..258, |frame| freed.push(frame));
        // Slot 256: PDPT, two PDs, one PT. Slot 257: PDPT.
        assert_eq!(count, 5);
        assert_eq!(freed.len(), 5);
        for (virt, _) in &pages[..3] {
            assert_eq!(mapper.translate(kernel(*virt)), None);
        }
        assert!(mapper.translate(kernel(pages[3].0)).is_some());
        assert!(!freed.contains(&mapper.root()));
    }
}
