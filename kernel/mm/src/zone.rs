//! Zone allocator for high-frequency, short-lived allocations.
//!
//! Provides power-of-two size classes (32 to 4096 bytes) backed by
//! pages obtained directly from the PMM via HHDM. Each zone maintains
//! an intrusive free list of fixed-size blocks.

use core::alloc::Layout;

use hadron_core::sync::SpinLock;

use crate::FrameAllocator;
use crate::hhdm;
use crate::pmm::{BitmapFrameAllocRef, with_pmm};

const NUM_ZONES: usize = 8;
const ZONE_SIZES: [usize; NUM_ZONES] = [32, 64, 128, 256, 512, 1024, 2048, 4096];
const PAGE_SIZE: usize = 4096;

/// Errors from zone allocation.
#[derive(Debug)]
pub enum ZoneAllocError {
    /// The requested size exceeds the maximum zone block size.
    TooLarge,
    /// Out of physical memory.
    OutOfMemory,
}

/// Free block header within a zone (intrusive linked list).
#[repr(C)]
struct FreeZoneBlock {
    next: *mut FreeZoneBlock,
}

struct Zone {
    block_size: usize,
    free_head: *mut FreeZoneBlock,
    pages_allocated: usize,
}

// SAFETY: Zone is accessed only under SpinLock.
unsafe impl Send for Zone {}

impl Zone {
    const fn new(block_size: usize) -> Self {
        Self {
            block_size,
            free_head: core::ptr::null_mut(),
            pages_allocated: 0,
        }
    }

    /// Allocates a block from this zone, requesting a new page if the free
    /// list is empty.
    fn alloc(&mut self) -> Result<*mut u8, ZoneAllocError> {
        if self.free_head.is_null() {
            self.grow()?;
        }

        let block = self.free_head;
        self.free_head = unsafe { (*block).next };
        Ok(block as *mut u8)
    }

    /// Returns a block to this zone's free list.
    ///
    /// # Safety
    /// `ptr` must have been allocated from this zone.
    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        let block = ptr as *mut FreeZoneBlock;
        unsafe {
            (*block).next = self.free_head;
        }
        self.free_head = block;
    }

    /// Allocates a new page from the PMM and carves it into blocks.
    fn grow(&mut self) -> Result<(), ZoneAllocError> {
        with_pmm(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            let frame = alloc.allocate_frame().ok_or(ZoneAllocError::OutOfMemory)?;

            let virt = hhdm::phys_to_virt(frame.start_address());
            let page_ptr = virt.as_u64() as *mut u8;

            // Zero the page.
            unsafe {
                core::ptr::write_bytes(page_ptr, 0, PAGE_SIZE);
            }

            // Carve page into blocks.
            let blocks_per_page = PAGE_SIZE / self.block_size;
            for i in (0..blocks_per_page).rev() {
                let block = unsafe { page_ptr.add(i * self.block_size) as *mut FreeZoneBlock };
                unsafe {
                    (*block).next = self.free_head;
                }
                self.free_head = block;
            }

            self.pages_allocated += 1;
            Ok(())
        })
    }
}

/// Power-of-two zone allocator.
pub struct ZoneAllocator {
    zones: [SpinLock<Zone>; NUM_ZONES],
}

impl ZoneAllocator {
    /// Creates a new zone allocator.
    pub const fn new() -> Self {
        Self {
            zones: [
                SpinLock::new(Zone::new(32)),
                SpinLock::new(Zone::new(64)),
                SpinLock::new(Zone::new(128)),
                SpinLock::new(Zone::new(256)),
                SpinLock::new(Zone::new(512)),
                SpinLock::new(Zone::new(1024)),
                SpinLock::new(Zone::new(2048)),
                SpinLock::new(Zone::new(4096)),
            ],
        }
    }

    /// Allocates a block matching the given layout.
    ///
    /// The size is rounded up to the nearest power-of-two zone (min 32).
    /// Allocations > 4096 bytes are rejected.
    pub fn alloc(&self, layout: Layout) -> Result<*mut u8, ZoneAllocError> {
        let size = layout.size().max(layout.align()).max(32);
        let zone_idx = self.zone_index(size).ok_or(ZoneAllocError::TooLarge)?;
        self.zones[zone_idx].lock().alloc()
    }

    /// Deallocates a block.
    ///
    /// # Safety
    /// `ptr` must have been allocated by this zone allocator with the same layout.
    pub unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let size = layout.size().max(layout.align()).max(32);
        if let Some(zone_idx) = self.zone_index(size) {
            unsafe { self.zones[zone_idx].lock().dealloc(ptr) };
        }
    }

    /// Returns the zone index for a given size.
    fn zone_index(&self, size: usize) -> Option<usize> {
        ZONE_SIZES.iter().position(|&s| s >= size)
    }
}

/// Global zone allocator instance.
static ZONE_ALLOCATOR: ZoneAllocator = ZoneAllocator::new();

/// Allocates a block from the global zone allocator.
pub fn zone_alloc(layout: Layout) -> Result<*mut u8, ZoneAllocError> {
    ZONE_ALLOCATOR.alloc(layout)
}

/// Deallocates a block from the global zone allocator.
///
/// # Safety
/// `ptr` must have been allocated by `zone_alloc` with the same layout.
pub unsafe fn zone_dealloc(ptr: *mut u8, layout: Layout) {
    unsafe { ZONE_ALLOCATOR.dealloc(ptr, layout) };
}
