//! Memory syscall handlers: mem_map, mem_unmap.
//!
//! Implements anonymous memory mapping for userspace processes. Each process
//! owns a [`FreeRegionAllocator`](crate::mm::region::FreeRegionAllocator)
//! that tracks the mmap virtual address region. Physical frames are allocated
//! from the PMM and mapped into the process's address space.

use crate::addr::VirtAddr;
use crate::mm::PAGE_SIZE;
use crate::mm::mapper::MapFlags;
use crate::mm::pmm::{BitmapFrameAllocRef, with_pmm};
use crate::paging::{Page, Size4KiB};
use crate::proc::with_current_process;
use crate::syscall::{EINVAL, ENOSYS};

/// Page-align `size` upward (round to next 4 KiB boundary).
const fn page_align_up(size: usize) -> usize {
    (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// `sys_mem_map` — map anonymous memory into the calling process's address space.
///
/// `addr_hint` is currently ignored (kernel always chooses the address).
/// `length` is rounded up to page alignment. `prot` is a bitmask of
/// `PROT_READ`/`PROT_WRITE`/`PROT_EXEC`. `flags` must include `MAP_ANONYMOUS`.
///
/// Returns the mapped virtual address on success, or negated errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning virtual address as isize; upper bit is never set for user addresses"
)]
pub(super) fn sys_mem_map(
    _addr_hint: usize,
    length: usize,
    prot: usize,
    flags: usize,
) -> isize {
    use hadron_syscall::{MAP_ANONYMOUS, PROT_EXEC, PROT_READ, PROT_WRITE};

    // Validate flags.
    if flags & MAP_ANONYMOUS == 0 {
        return -ENOSYS; // Only anonymous mappings supported.
    }
    if length == 0 {
        return -EINVAL;
    }

    let aligned_length = page_align_up(length);
    let page_count = aligned_length / PAGE_SIZE;

    // Build page table flags from prot.
    let mut map_flags = MapFlags::USER;
    if prot & PROT_WRITE != 0 {
        map_flags |= MapFlags::WRITABLE;
    }
    if prot & PROT_EXEC != 0 {
        map_flags |= MapFlags::EXECUTABLE;
    }
    // PROT_READ is implicit (all mapped pages are readable on x86_64).
    let _ = prot & PROT_READ;

    // Allocate virtual region from the process's mmap allocator.
    let vaddr = with_current_process(|process| {
        let mut mmap = process.mmap_alloc.lock();
        mmap.allocate(aligned_length as u64)
    });

    let base_vaddr = match vaddr {
        Some(v) => v,
        None => return -EINVAL, // Region exhausted.
    };

    // Allocate physical frames and map pages.
    let hhdm_offset = crate::mm::hhdm::offset();
    let map_result = with_current_process(|process| {
        with_pmm(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            for i in 0..page_count {
                let page_vaddr = base_vaddr.as_u64() + (i as u64) * PAGE_SIZE as u64;
                let frame = match alloc.0.allocate_frame() {
                    Some(f) => f,
                    None => return Err(i), // Out of memory — need to unwind.
                };

                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                if let Err(_e) =
                    process
                        .address_space()
                        .map_user_page(page, frame, map_flags, &mut alloc)
                {
                    return Err(i);
                }

                // Zero the page via HHDM.
                let frame_ptr = (hhdm_offset + frame.start_address().as_u64()) as *mut u8;
                // SAFETY: Frame was just allocated; zeroing via HHDM is safe.
                unsafe {
                    core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);
                }
            }
            Ok(())
        })
    });

    if let Err(_partial_count) = map_result {
        // TODO: Unmap partially-mapped pages. For now, leak them.
        return -EINVAL;
    }

    base_vaddr.as_u64() as isize
}

/// `sys_mem_unmap` — unmap previously mapped memory from the process's address space.
///
/// `addr` must be the exact address returned by `mem_map`. `length` must match
/// the original mapping size.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning 0 on success as isize"
)]
pub(super) fn sys_mem_unmap(addr: usize, length: usize) -> isize {
    if length == 0 || addr == 0 {
        return -EINVAL;
    }

    let aligned_length = page_align_up(length);
    let page_count = aligned_length / PAGE_SIZE;
    let base = VirtAddr::new(addr as u64);

    with_current_process(|process| {
        // Unmap pages and free physical frames.
        with_pmm(|pmm| {
            for i in 0..page_count {
                let page_vaddr = base.as_u64() + (i as u64) * PAGE_SIZE as u64;
                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                if let Ok(frame) = process.address_space().unmap_user_page(page) {
                    // SAFETY: The frame was allocated by the PMM during mem_map
                    // and is no longer referenced by any page table entry.
                    let _ = unsafe { pmm.deallocate_frame(frame) };
                }
            }
        });

        // Return the virtual region to the mmap allocator.
        let mut mmap = process.mmap_alloc.lock();
        let _ = mmap.deallocate(base, aligned_length as u64);
    });

    0
}
