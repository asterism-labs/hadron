//! Fake physical memory for host tests.
//!
//! Physical frame `n` is the `n`-th table of a heap allocation, so the
//! direct map offset is the allocation's address.

use arch_x86_64::PhysAddr;
use arch_x86_64::paging::{PAGE_SIZE, PageTable};

use crate::direct_map::DirectMap;
use crate::frame::{FrameAllocator, PhysFrame};
use crate::mapper::Mapper;

/// `frames` zeroed frames that hand themselves out in order.
pub(crate) struct FakeMemory {
    frames: Vec<PageTable>,
    next: usize,
}

impl FakeMemory {
    pub(crate) fn new(frames: usize) -> Self {
        Self {
            frames: vec![PageTable::new(); frames],
            next: 0,
        }
    }

    pub(crate) fn direct_map(&self) -> DirectMap {
        DirectMap::new(self.frames.as_ptr() as u64)
    }

    /// A mapper over a new, empty tree in this memory.
    pub(crate) fn mapper(&mut self) -> Mapper {
        // SAFETY: every frame this allocator hands out is in `frames`,
        // which the direct map covers, and the tests drop the mapper
        // before `frames`.
        unsafe { Mapper::empty(self.direct_map(), self).unwrap() }
    }

    /// Frames handed out so far.
    pub(crate) fn tables_used(&self) -> usize {
        self.next
    }

    #[allow(clippy::cast_possible_truncation)]
    pub(crate) fn table(&self, addr: PhysAddr) -> &PageTable {
        &self.frames[(addr.as_u64() / PAGE_SIZE) as usize]
    }
}

impl FrameAllocator for FakeMemory {
    fn allocate_frame(&mut self) -> Option<PhysFrame> {
        if self.next == self.frames.len() {
            return None;
        }
        self.next += 1;
        Some(PhysFrame::from_number(self.next as u64 - 1))
    }
}
