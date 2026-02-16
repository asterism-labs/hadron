//! Physical memory manager -- kernel-side glue.
//!
//! Wraps the core [`BitmapAllocator`] in a global static and provides
//! initialization from boot info.

use hadron_core::addr::PhysAddr;
use hadron_core::mm::pmm::BitmapAllocator;
use hadron_core::mm::PhysMemoryRegion;
use hadron_core::sync::SpinLock;

use crate::boot::{BootInfo, MemoryRegionKind};

/// Global physical memory manager.
static PMM: SpinLock<Option<BitmapAllocator>> = SpinLock::new(None);

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

    let allocator = unsafe {
        BitmapAllocator::new(&regions[..count], hhdm_offset)
            .expect("failed to initialize PMM")
    };

    let mut pmm = PMM.lock();
    assert!(pmm.is_none(), "PMM already initialized");
    *pmm = Some(allocator);
}

/// Executes a closure with a reference to the global PMM.
///
/// # Panics
///
/// Panics if the PMM has not been initialized.
pub fn with_pmm<R>(f: impl FnOnce(&BitmapAllocator) -> R) -> R {
    let pmm = PMM.lock();
    f(pmm.as_ref().expect("PMM not initialized"))
}

/// Attempts to execute a closure with a reference to the global PMM.
///
/// Returns `None` if the PMM lock is already held (avoiding deadlock in
/// fault handlers) or if the PMM has not been initialized yet.
pub fn try_with_pmm<R>(f: impl FnOnce(&BitmapAllocator) -> R) -> Option<R> {
    let pmm = PMM.try_lock()?;
    Some(f(pmm.as_ref()?))
}
