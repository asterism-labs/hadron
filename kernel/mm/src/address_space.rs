//! User address space management.
//!
//! Each process owns an [`AddressSpace`] that holds a per-process PML4
//! with the kernel upper half copied from the kernel root page table.
//! User pages are mapped into the lower half (entries 0–255).

use hadron_core::addr::{PhysAddr, VirtAddr};
use hadron_core::paging::{Page, PhysFrame, Size4KiB};

use crate::mapper::{MapFlags, MapFlush, PageMapper, PageTranslator, UnmapError};
use crate::{FrameAllocator, VmmError};

/// Number of PML4 entries in the upper half (indices 256–511).
const KERNEL_PML4_ENTRIES: usize = 256;

/// Callback for deallocating a single physical frame.
///
/// Stored at construction time so that `Drop` can free the PML4 frame
/// without needing access to a `FrameDeallocator` parameter.
pub type FrameDeallocFn = fn(PhysFrame<Size4KiB>);

/// A user-mode address space backed by its own PML4.
///
/// The upper half (PML4 entries 256–511) is shared with the kernel;
/// the lower half (entries 0–255) is process-private.
///
/// On drop, the PML4 frame is freed via the stored deallocation callback.
pub struct AddressSpace<M: PageMapper<Size4KiB> + PageTranslator> {
    /// Physical address of this address space's PML4 frame.
    root_phys: PhysAddr,
    /// Page table mapper (shared, knows HHDM offset).
    mapper: M,
    /// Callback to free physical frames on drop.
    dealloc_fn: FrameDeallocFn,
}

impl<M: PageMapper<Size4KiB> + PageTranslator> AddressSpace<M> {
    /// Creates a new user address space.
    ///
    /// Allocates a fresh PML4 frame and copies the kernel upper half
    /// (entries 256–511) from `kernel_root`. The lower half is zeroed.
    ///
    /// `dealloc_fn` is stored and called in `Drop` to free the PML4 frame.
    ///
    /// # Safety
    ///
    /// `kernel_root` must point to a valid PML4 used by the kernel.
    /// `alloc` must return zeroed 4 KiB frames.
    pub unsafe fn new_user(
        kernel_root: PhysAddr,
        mapper: M,
        hhdm_offset: u64,
        alloc: &mut impl FrameAllocator<Size4KiB>,
        dealloc_fn: FrameDeallocFn,
    ) -> Result<Self, VmmError> {
        let frame = alloc.allocate_frame().ok_or(VmmError::OutOfMemory)?;
        let new_pml4_phys = frame.start_address();

        // SAFETY: The frames are accessible via HHDM. We zero the user half
        // and copy the kernel half.
        unsafe {
            let new_pml4 = (hhdm_offset + new_pml4_phys.as_u64()) as *mut u64;
            let kernel_pml4 = (hhdm_offset + kernel_root.as_u64()) as *const u64;

            // Zero the lower half (entries 0–255).
            core::ptr::write_bytes(new_pml4, 0, KERNEL_PML4_ENTRIES);

            // Copy the upper half (entries 256–511) from the kernel PML4.
            core::ptr::copy_nonoverlapping(
                kernel_pml4.add(KERNEL_PML4_ENTRIES),
                new_pml4.add(KERNEL_PML4_ENTRIES),
                KERNEL_PML4_ENTRIES,
            );
        }

        Ok(Self {
            root_phys: new_pml4_phys,
            mapper,
            dealloc_fn,
        })
    }

    /// Maps a single 4 KiB page into the user address space.
    ///
    /// The `USER` flag is always added to `flags`.
    ///
    /// Returns a [`MapFlush`] that the caller must handle.
    pub fn map_user_page(
        &self,
        page: Page<Size4KiB>,
        frame: PhysFrame<Size4KiB>,
        flags: MapFlags,
        alloc: &mut impl FrameAllocator<Size4KiB>,
    ) -> Result<MapFlush, VmmError> {
        let flags = flags | MapFlags::USER;
        // SAFETY: The AddressSpace owns its PML4 (root_phys). The caller
        // provides a valid physical frame and allocator for page table pages.
        let flush = unsafe {
            self.mapper
                .map(self.root_phys, page, frame, flags, &mut || {
                    alloc
                        .allocate_frame()
                        .expect("PMM: out of memory during user map")
                })
        };
        Ok(flush)
    }

    /// Unmaps a single 4 KiB page from the user address space.
    ///
    /// Flushes the TLB internally and returns the freed frame.
    pub fn unmap_user_page(&self, page: Page<Size4KiB>) -> Result<PhysFrame<Size4KiB>, VmmError> {
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

    /// Returns the physical address of this address space's PML4.
    ///
    /// Used for loading into CR3 on context switch.
    pub fn root_phys(&self) -> PhysAddr {
        self.root_phys
    }

    /// Translates a virtual address within this address space.
    pub fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        unsafe { <M as PageTranslator>::translate_addr(&self.mapper, self.root_phys, virt) }
    }
}

impl<M: PageMapper<Size4KiB> + PageTranslator> Drop for AddressSpace<M> {
    fn drop(&mut self) {
        let frame = PhysFrame::containing_address(self.root_phys);
        (self.dealloc_fn)(frame);
    }
}
