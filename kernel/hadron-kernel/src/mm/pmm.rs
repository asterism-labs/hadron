//! Physical memory manager — kernel glue.
//!
//! Re-exports the bitmap allocator and PMM global from `hadron-mm`.
//! Adds `init(boot_info)` which converts bootloader memory map into
//! `PhysMemoryRegion` descriptors before delegating to `hadron_mm::pmm::init`.

pub use hadron_mm::pmm::*;

use crate::addr::PhysAddr;
use crate::boot::{BootInfo, MemoryRegionKind};

use hadron_mm::PhysMemoryRegion;

/// Initializes the PMM from boot info.
///
/// Converts the bootloader memory map into `PhysMemoryRegion` descriptors
/// and creates the bitmap allocator.
pub fn init(boot_info: &impl BootInfo) {
    let hhdm_offset = boot_info.hhdm_offset();
    let memory_map = boot_info.memory_map();

    // Convert boot info regions to PhysMemoryRegion.
    // Use a stack buffer since we can't heap-allocate yet.
    let mut regions = [PhysMemoryRegion {
        start: PhysAddr::zero(),
        size: 0,
        usable: false,
    }; 256];
    let mut count = 0;

    for region in memory_map {
        if count >= regions.len() {
            break;
        }
        regions[count] = PhysMemoryRegion {
            start: region.start,
            size: region.size,
            usable: region.kind == MemoryRegionKind::Usable,
        };
        count += 1;
    }

    hadron_mm::pmm::init(&regions[..count], hhdm_offset);
}
