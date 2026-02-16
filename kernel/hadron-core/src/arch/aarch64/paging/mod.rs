//! AArch64 page table management (stub).

use crate::addr::{PhysAddr, VirtAddr};
use crate::mm::mapper::{self, MapFlags, MapFlush, UnmapError};
use crate::paging::{Page, PhysFrame, Size4KiB};

/// AArch64 page table mapper (stub).
pub struct AArch64PageMapper {
    #[allow(dead_code)] // Phase: aarch64 bring-up
    hhdm_offset: u64,
}

impl AArch64PageMapper {
    /// Creates a new mapper with the given HHDM offset.
    pub fn new(hhdm_offset: u64) -> Self {
        Self { hhdm_offset }
    }
}

// SAFETY: stub — all methods `todo!()`.
unsafe impl mapper::PageMapper<Size4KiB> for AArch64PageMapper {
    unsafe fn map(
        &self,
        _root: PhysAddr,
        _page: Page<Size4KiB>,
        _frame: PhysFrame<Size4KiB>,
        _flags: MapFlags,
        _alloc: &mut dyn FnMut() -> PhysFrame<Size4KiB>,
    ) -> MapFlush {
        todo!("aarch64 map 4KiB")
    }

    unsafe fn unmap(
        &self,
        _root: PhysAddr,
        _page: Page<Size4KiB>,
    ) -> Result<(PhysFrame<Size4KiB>, MapFlush), UnmapError> {
        todo!("aarch64 unmap 4KiB")
    }

    unsafe fn update_flags(
        &self,
        _root: PhysAddr,
        _page: Page<Size4KiB>,
        _flags: MapFlags,
    ) -> Result<MapFlush, UnmapError> {
        todo!("aarch64 update_flags 4KiB")
    }
}

// SAFETY: stub — `todo!()`.
unsafe impl mapper::PageTranslator for AArch64PageMapper {
    unsafe fn translate_addr(
        &self,
        _root: PhysAddr,
        _virt: VirtAddr,
    ) -> Option<PhysAddr> {
        todo!("aarch64 translate_addr")
    }
}
