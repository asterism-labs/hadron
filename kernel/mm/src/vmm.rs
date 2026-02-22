//! Virtual memory manager for kernel address space.
//!
//! Manages page table mappings and virtual address allocation for kernel
//! regions (heap, stacks, MMIO). Uses [`RegionAllocator`] for the heap
//! (bump-only) and [`FreeRegionAllocator`] for stacks and MMIO (with
//! deallocation support). A [`PageMapper`] implementation handles page
//! table manipulation.

use hadron_core::addr::{PhysAddr, VirtAddr};

use crate::layout::{INITIAL_HEAP_SIZE, MemoryLayout};
use crate::mapper::{MapFlags, MapFlush, PageMapper, PageTranslator, UnmapError};
use crate::region::{FreeRegionAllocator, RegionAllocator};
use crate::{FrameAllocator, PAGE_SIZE, VmmError};
use hadron_core::paging::{Page, PhysFrame, Size4KiB};

/// Default kernel stack size: 64 KiB (16 pages).
const KERNEL_STACK_SIZE: u64 = 64 * 1024;
/// Guard page size: one page, unmapped.
const GUARD_PAGE_SIZE: u64 = PAGE_SIZE as u64;

/// Callback for kernel stack cleanup on drop.
pub type StackCleanupFn = fn(guard: VirtAddr, bottom: VirtAddr, top: VirtAddr);

/// A kernel stack with a guard page.
///
/// When dropped, calls the cleanup callback (if set) to unmap pages and
/// free physical frames.
#[derive(Debug)]
pub struct KernelStack {
    top: VirtAddr,
    bottom: VirtAddr,
    guard: VirtAddr,
    cleanup: Option<StackCleanupFn>,
}

impl KernelStack {
    /// Top of the stack (highest address, where SP starts).
    #[must_use]
    pub fn top(&self) -> VirtAddr {
        self.top
    }

    /// Bottom of the usable stack (lowest mapped address).
    #[must_use]
    pub fn bottom(&self) -> VirtAddr {
        self.bottom
    }

    /// Guard page address (unmapped, below bottom).
    #[must_use]
    pub fn guard(&self) -> VirtAddr {
        self.guard
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup {
            (cleanup)(self.guard, self.bottom, self.top);
        }
    }
}

/// Callback for MMIO mapping cleanup on drop.
pub type MmioCleanupFn = fn(virt_base: VirtAddr, size: u64);

/// An MMIO mapping.
///
/// When dropped, calls the cleanup callback (if set) to unmap pages.
#[derive(Debug)]
pub struct MmioMapping {
    virt_base: VirtAddr,
    phys_base: PhysAddr,
    size: u64,
    cleanup: Option<MmioCleanupFn>,
}

impl MmioMapping {
    /// Virtual base address.
    #[must_use]
    pub fn virt_base(&self) -> VirtAddr {
        self.virt_base
    }

    /// Physical base address.
    #[must_use]
    pub fn phys_base(&self) -> PhysAddr {
        self.phys_base
    }

    /// Size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }
}

impl Drop for MmioMapping {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup {
            (cleanup)(self.virt_base, self.size);
        }
    }
}

/// Maximum number of disjoint free ranges in the stacks region allocator.
const STACKS_FREE_LIST_CAP: usize = 256;

/// Maximum number of disjoint free ranges in the MMIO region allocator.
const MMIO_FREE_LIST_CAP: usize = 128;

/// The kernel virtual memory manager, generic over the page mapper.
pub struct Vmm<M: PageMapper<Size4KiB> + PageTranslator> {
    root_phys: PhysAddr,
    mapper: M,
    layout: MemoryLayout,
    heap_alloc: RegionAllocator,
    stacks_alloc: FreeRegionAllocator<STACKS_FREE_LIST_CAP>,
    mmio_alloc: FreeRegionAllocator<MMIO_FREE_LIST_CAP>,
}

impl<M: PageMapper<Size4KiB> + PageTranslator> Vmm<M> {
    /// Creates a new VMM wrapping the given root page table.
    pub fn new(root_phys: PhysAddr, mapper: M, hhdm_offset: u64, max_phys: u64) -> Self {
        let layout = MemoryLayout::new(hhdm_offset, max_phys);
        Self {
            root_phys,
            mapper,
            heap_alloc: RegionAllocator::new(layout.heap),
            stacks_alloc: FreeRegionAllocator::new(layout.stacks),
            mmio_alloc: FreeRegionAllocator::new(layout.mmio),
            layout,
        }
    }

    /// Returns a reference to the memory layout.
    pub fn layout(&self) -> &MemoryLayout {
        &self.layout
    }

    /// Returns the current heap allocation watermark (next unallocated address).
    pub fn heap_watermark(&self) -> VirtAddr {
        self.heap_alloc.current()
    }

    /// Returns the current stacks allocation watermark (next unallocated address).
    pub fn stacks_watermark(&self) -> VirtAddr {
        self.stacks_alloc.watermark()
    }

    /// Maps the initial kernel heap region (4 MiB by default).
    ///
    /// Returns `(base_address, size_in_bytes)`.
    pub fn map_initial_heap(
        &mut self,
        alloc: &mut impl FrameAllocator<Size4KiB>,
    ) -> Result<(VirtAddr, u64), VmmError> {
        self.grow_heap(INITIAL_HEAP_SIZE, alloc)
    }

    /// Grows the kernel heap by the given number of bytes (rounded to pages).
    ///
    /// Returns `(base_address_of_new_pages, actual_bytes_mapped)`.
    pub fn grow_heap(
        &mut self,
        bytes: u64,
        alloc: &mut impl FrameAllocator<Size4KiB>,
    ) -> Result<(VirtAddr, u64), VmmError> {
        let page_size = PAGE_SIZE as u64;
        let page_count = (bytes + page_size - 1) / page_size;
        let actual_bytes = page_count * page_size;

        let base = self
            .heap_alloc
            .allocate(actual_bytes)
            .ok_or(VmmError::RegionExhausted)?;

        let flags = MapFlags::WRITABLE | MapFlags::GLOBAL;

        for i in 0..page_count {
            let virt = base + i * page_size;
            let page = Page::containing_address(virt);
            let frame = alloc.allocate_frame().ok_or(VmmError::OutOfMemory)?;
            // SAFETY: The mapper guarantees that mapping a page within an allocated
            // region with a valid frame is correct. The frame allocator closure
            // provides page table frames as needed.
            let flush = unsafe {
                self.mapper
                    .map(self.root_phys, page, frame, flags, &mut || {
                        alloc
                            .allocate_frame()
                            .expect("PMM: out of memory during heap grow")
                    })
            };
            // Fresh mapping, never in TLB.
            flush.ignore();
            // SAFETY: `virt` was just mapped to a valid physical frame; zeroing
            // the page initialises it for heap use.
            unsafe {
                core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, PAGE_SIZE);
            }
        }

        Ok((base, actual_bytes))
    }

    /// Allocates and maps a kernel stack with a guard page.
    ///
    /// `cleanup` is called when the `KernelStack` is dropped. Pass `None`
    /// for the BSP boot stack or stacks that outlive the kernel.
    pub fn alloc_kernel_stack(
        &mut self,
        alloc: &mut impl FrameAllocator<Size4KiB>,
        cleanup: Option<StackCleanupFn>,
    ) -> Result<KernelStack, VmmError> {
        let total_size = GUARD_PAGE_SIZE + KERNEL_STACK_SIZE;
        let base = self
            .stacks_alloc
            .allocate(total_size)
            .ok_or(VmmError::RegionExhausted)?;

        // Guard page is the first page -- left unmapped.
        let stack_bottom = base + GUARD_PAGE_SIZE;
        let stack_top = stack_bottom + KERNEL_STACK_SIZE;

        let flags = MapFlags::WRITABLE | MapFlags::GLOBAL;

        let page_size = PAGE_SIZE as u64;
        let stack_pages = KERNEL_STACK_SIZE / page_size;
        for i in 0..stack_pages {
            let virt = stack_bottom + i * page_size;
            let page = Page::containing_address(virt);
            let frame = alloc.allocate_frame().ok_or(VmmError::OutOfMemory)?;
            // SAFETY: Same as grow_heap — mapping within an allocated region.
            let flush = unsafe {
                self.mapper
                    .map(self.root_phys, page, frame, flags, &mut || {
                        alloc
                            .allocate_frame()
                            .expect("PMM: out of memory during stack alloc")
                    })
            };
            // Fresh mapping, never in TLB.
            flush.ignore();
            // SAFETY: Zeroing the freshly-mapped page is safe.
            unsafe {
                core::ptr::write_bytes(virt.as_mut_ptr::<u8>(), 0, PAGE_SIZE);
            }
        }

        Ok(KernelStack {
            top: stack_top,
            bottom: stack_bottom,
            guard: base,
            cleanup,
        })
    }

    /// Maps a physical MMIO region into kernel virtual address space.
    ///
    /// `cleanup` is called when the `MmioMapping` is dropped. Pass `None`
    /// for permanent mappings.
    pub fn map_mmio(
        &mut self,
        phys: PhysAddr,
        size: u64,
        alloc: &mut impl FrameAllocator<Size4KiB>,
        cleanup: Option<MmioCleanupFn>,
    ) -> Result<MmioMapping, VmmError> {
        let page_size = PAGE_SIZE as u64;
        let page_count = (size + page_size - 1) / page_size;
        let actual_size = page_count * page_size;

        let virt_base = self
            .mmio_alloc
            .allocate(actual_size)
            .ok_or(VmmError::RegionExhausted)?;

        let flags = MapFlags::WRITABLE | MapFlags::GLOBAL | MapFlags::CACHE_DISABLE;

        for i in 0..page_count {
            let virt = virt_base + i * page_size;
            let page = Page::containing_address(virt);
            let phys_page = PhysFrame::containing_address(phys + i * page_size);
            // SAFETY: The MMIO physical address is provided by firmware (ACPI).
            // Mapping it into the MMIO region with cache-disable flags is correct
            // for device register access.
            let flush = unsafe {
                self.mapper
                    .map(self.root_phys, page, phys_page, flags, &mut || {
                        alloc
                            .allocate_frame()
                            .expect("PMM: out of memory during MMIO map")
                    })
            };
            // Fresh mapping, never in TLB.
            flush.ignore();
        }

        Ok(MmioMapping {
            virt_base,
            phys_base: phys,
            size: actual_size,
            cleanup,
        })
    }

    /// Maps a single 4 KiB page.
    ///
    /// Returns a [`MapFlush`] that the caller must handle (flush or ignore).
    pub fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        frame: PhysFrame<Size4KiB>,
        flags: MapFlags,
        alloc: &mut impl FrameAllocator<Size4KiB>,
    ) -> Result<MapFlush, VmmError> {
        // SAFETY: The Vmm owns the root page table and the caller provides
        // a valid physical frame and allocator.
        let flush = unsafe {
            self.mapper
                .map(self.root_phys, page, frame, flags, &mut || {
                    alloc
                        .allocate_frame()
                        .expect("PMM: out of memory during map_page")
                })
        };
        Ok(flush)
    }

    /// Unmaps a single 4 KiB page, flushes the TLB, and returns the frame.
    pub fn unmap_page(&mut self, page: Page<Size4KiB>) -> Result<PhysFrame<Size4KiB>, VmmError> {
        // SAFETY: The Vmm owns the root page table. Unmapping returns the
        // previously-mapped frame for the caller to deallocate.
        let (frame, flush) = unsafe {
            self.mapper
                .unmap(self.root_phys, page)
                .map_err(|e| match e {
                    UnmapError::NotMapped => VmmError::NotMapped,
                    UnmapError::SizeMismatch => VmmError::SizeMismatch,
                })?
        };
        flush.flush();
        Ok(frame)
    }

    /// Translates a virtual address to a physical address.
    pub fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        // SAFETY: The Vmm owns root_phys; a read-only page table walk is safe.
        unsafe { <M as PageTranslator>::translate_addr(&self.mapper, self.root_phys, virt) }
    }

    /// Returns a previously allocated stack region to the stacks allocator.
    ///
    /// `guard_addr` is the base of the guard page (== `KernelStack::guard()`).
    /// The total size (guard + stack) is computed automatically.
    pub fn dealloc_stack_region(&mut self, guard_addr: VirtAddr) -> Result<(), VmmError> {
        let total_size = GUARD_PAGE_SIZE + KERNEL_STACK_SIZE;
        self.stacks_alloc
            .deallocate(guard_addr, total_size)
            .map_err(|_| VmmError::RegionExhausted)
    }

    /// Returns a previously allocated MMIO region to the MMIO allocator.
    pub fn dealloc_mmio_region(&mut self, virt_base: VirtAddr, size: u64) -> Result<(), VmmError> {
        self.mmio_alloc
            .deallocate(virt_base, size)
            .map_err(|_| VmmError::RegionExhausted)
    }
}
