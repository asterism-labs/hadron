//! Memory syscall handlers: mem_map, mem_unmap, mem_brk, mem_create_shared, mem_map_shared.
//!
//! Implements anonymous, device-backed, and shared memory mapping for userspace
//! processes. Each process owns a
//! [`FreeRegionAllocator`](crate::mm::region::FreeRegionAllocator) that tracks
//! the mmap virtual address region. Physical frames are allocated from the PMM
//! (anonymous) or come from device MMIO regions (device-backed) or shared
//! memory objects (shared).

use crate::addr::VirtAddr;
use crate::fs::file::OpenFlags;
use crate::id::Fd;
use crate::mm::PAGE_SIZE;
use crate::mm::mapper::MapFlags;
use crate::mm::pmm::{self, BitmapFrameAllocRef};
use crate::paging::{Page, PhysFrame, Size4KiB};
use crate::proc::{MappingKind, ProcessTable};
use crate::syscall::{EBADF, EINVAL, ENOMEM, ENOSYS};

/// Page-align `size` upward (round to next 4 KiB boundary).
const fn page_align_up(size: usize) -> usize {
    (size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// `sys_mem_map` — map memory into the calling process's address space.
///
/// `addr_hint` is currently ignored (kernel always chooses the address).
/// `length` is rounded up to page alignment. `prot` is a bitmask of
/// `PROT_READ`/`PROT_WRITE`/`PROT_EXEC`. `flags` must include
/// `MAP_ANONYMOUS` or `MAP_SHARED`. `fd` is the file descriptor for
/// device-backed mappings (ignored for anonymous).
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
    fd: usize,
) -> isize {
    use hadron_syscall::{MAP_ANONYMOUS, MAP_SHARED};

    if length == 0 {
        return -EINVAL;
    }

    if flags & MAP_SHARED != 0 {
        return sys_mem_map_device(length, prot, fd);
    }

    if flags & MAP_ANONYMOUS != 0 {
        return sys_mem_map_anonymous(length, prot);
    }

    -ENOSYS // Neither MAP_ANONYMOUS nor MAP_SHARED.
}

/// Anonymous mapping: allocate physical frames from PMM.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning virtual address as isize; upper bit is never set for user addresses"
)]
fn sys_mem_map_anonymous(length: usize, prot: usize) -> isize {
    use hadron_syscall::{PROT_EXEC, PROT_READ, PROT_WRITE};

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
    let vaddr = ProcessTable::with_current(|process| {
        let mut mmap = process.mmap_alloc.lock();
        mmap.allocate(aligned_length as u64)
    });

    let base_vaddr = match vaddr {
        Some(v) => v,
        None => return -EINVAL, // Region exhausted.
    };

    // Allocate physical frames and map pages.
    let hhdm_offset = crate::mm::hhdm::offset();
    let map_result = ProcessTable::with_current(|process| {
        pmm::with(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            for i in 0..page_count {
                let page_vaddr = base_vaddr.as_u64() + (i as u64) * PAGE_SIZE as u64;
                let frame = match alloc.0.allocate_frame() {
                    Some(f) => f,
                    None => return Err(i), // Out of memory — need to unwind.
                };

                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                if let Err(_e) = process
                    .address_space()
                    .map_user_page(page, frame, map_flags, &mut alloc)
                {
                    return Err(i);
                }

                // Zero the page via HHDM.
                let frame_ptr = (hhdm_offset + frame.start_address().as_u64()).as_mut_ptr::<u8>();
                // SAFETY: Frame was just allocated; zeroing via HHDM is safe.
                unsafe {
                    core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);
                }
            }
            Ok(())
        })
    });

    if let Err(mapped_count) = map_result {
        // Unmap and free all pages that were successfully mapped before the failure.
        ProcessTable::with_current(|process| {
            pmm::with(|pmm| {
                for i in 0..mapped_count {
                    let page_vaddr = base_vaddr.as_u64() + (i as u64) * PAGE_SIZE as u64;
                    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                    if let Ok(frame) = process.address_space().unmap_user_page(page) {
                        // SAFETY: Frame was allocated by PMM during this call and is no
                        // longer referenced by any page table entry.
                        let _ = unsafe { pmm.deallocate_frame(frame) };
                    }
                }
            });
            // Return the virtual region to the mmap allocator.
            let mut mmap = process.mmap_alloc.lock();
            let _ = mmap.deallocate(base_vaddr, aligned_length as u64);
        });
        return -ENOMEM;
    }

    // Track this as an anonymous mapping.
    ProcessTable::with_current(|process| {
        let mut mappings = process.mmap_mappings.lock();
        mappings.insert(base_vaddr.as_u64(), MappingKind::Anonymous { page_count });
    });

    base_vaddr.as_u64() as isize
}

/// Device-backed mapping: map physical device memory into user space.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning virtual address as isize; upper bit is never set for user addresses"
)]
fn sys_mem_map_device(length: usize, prot: usize, fd: usize) -> isize {
    use hadron_syscall::{PROT_EXEC, PROT_READ, PROT_WRITE};

    let fd = Fd::new(fd as u32);

    // Look up the inode from the fd table.
    let inode = ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        fd_table.get(fd).map(|f| f.inode.clone())
    });

    let Some(inode) = inode else {
        return -EBADF;
    };

    // Query physical base and device size from the inode.
    let (phys_base, device_size) = match inode.mmap_phys() {
        Ok(v) => v,
        Err(e) => return -e.to_errno(),
    };

    let aligned_length = page_align_up(length);
    let page_count = aligned_length / PAGE_SIZE;

    if aligned_length > device_size {
        return -EINVAL;
    }

    // Build page table flags from prot. Use cache-disable for MMIO-backed
    // devices, write-back for RAM-backed ones (e.g. VirtIO GPU).
    let mut map_flags = MapFlags::USER;
    if inode.mmap_cache_disable() {
        map_flags |= MapFlags::CACHE_DISABLE;
    }
    if prot & PROT_WRITE != 0 {
        map_flags |= MapFlags::WRITABLE;
    }
    if prot & PROT_EXEC != 0 {
        map_flags |= MapFlags::EXECUTABLE;
    }
    let _ = prot & PROT_READ;

    // Allocate virtual region from the process's mmap allocator.
    let vaddr = ProcessTable::with_current(|process| {
        let mut mmap = process.mmap_alloc.lock();
        mmap.allocate(aligned_length as u64)
    });

    let base_vaddr = match vaddr {
        Some(v) => v,
        None => return -EINVAL,
    };

    // Map device physical pages into user address space.
    let map_result = ProcessTable::with_current(|process| {
        pmm::with(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            for i in 0..page_count {
                let page_vaddr = base_vaddr.as_u64() + (i as u64) * PAGE_SIZE as u64;
                let phys_addr = phys_base + (i as u64) * PAGE_SIZE as u64;

                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                let frame = PhysFrame::<Size4KiB>::containing_address(phys_addr);

                if let Err(_e) = process
                    .address_space()
                    .map_user_page(page, frame, map_flags, &mut alloc)
                {
                    return Err(i);
                }
            }
            Ok(())
        })
    });

    if let Err(mapped_count) = map_result {
        // Unmap PTEs for all pages that were successfully mapped. Device frames
        // are not freed — they belong to the hardware.
        ProcessTable::with_current(|process| {
            for i in 0..mapped_count {
                let page_vaddr = base_vaddr.as_u64() + (i as u64) * PAGE_SIZE as u64;
                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                let _ = process.address_space().unmap_user_page(page);
            }
            // Return the virtual region to the mmap allocator.
            let mut mmap = process.mmap_alloc.lock();
            let _ = mmap.deallocate(base_vaddr, aligned_length as u64);
        });
        return -ENOMEM;
    }

    // Track this as a device mapping (physical frames must NOT be freed).
    ProcessTable::with_current(|process| {
        let mut mappings = process.mmap_mappings.lock();
        mappings.insert(base_vaddr.as_u64(), MappingKind::Device { page_count });
    });

    base_vaddr.as_u64() as isize
}

/// `sys_mem_unmap` — unmap previously mapped memory from the process's address space.
///
/// `addr` must be the exact address returned by `mem_map`. `length` must match
/// the original mapping size.
#[expect(clippy::cast_possible_wrap, reason = "returning 0 on success as isize")]
pub(super) fn sys_mem_unmap(addr: usize, length: usize) -> isize {
    if length == 0 || addr == 0 {
        return -EINVAL;
    }

    let aligned_length = page_align_up(length);
    let base = VirtAddr::new(addr as u64);

    ProcessTable::with_current(|process| {
        // Look up the mapping kind to decide whether to free frames.
        let mapping_kind = {
            let mut mappings = process.mmap_mappings.lock();
            mappings.remove(&base.as_u64())
        };

        let page_count = aligned_length / PAGE_SIZE;

        match mapping_kind {
            Some(MappingKind::Device { .. } | MappingKind::Shared { .. }) => {
                // Device/shared mapping: unmap PTEs but do NOT free physical frames.
                // Device frames belong to hardware; shared frames are owned by
                // the ShmObject and freed when its last Arc ref is dropped.
                for i in 0..page_count {
                    let page_vaddr = base.as_u64() + (i as u64) * PAGE_SIZE as u64;
                    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                    let _ = process.address_space().unmap_user_page(page);
                }
            }
            _ => {
                // Anonymous (or legacy untracked): unmap and free frames.
                pmm::with(|pmm| {
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
            }
        }

        // Return the virtual region to the mmap allocator.
        let mut mmap = process.mmap_alloc.lock();
        let _ = mmap.deallocate(base, aligned_length as u64);
    });

    0
}

/// `sys_mem_brk` — adjust the program break (heap boundary).
///
/// If `addr` is 0, returns the current break address.
/// If `addr > current_break`, the heap is expanded by mapping new pages.
/// If `addr < current_break`, the heap is shrunk by unmapping pages.
///
/// Returns the new break address on success, or negated errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning virtual address as isize; upper bit is never set for user addresses"
)]
pub(super) fn sys_mem_brk(addr: usize) -> isize {
    let addr = addr as u64;

    ProcessTable::with_current(|process| {
        let mut brk = process.program_break.lock();
        let current = *brk;

        // Query current break.
        if addr == 0 {
            return current as isize;
        }

        let new_brk = page_align_up(addr as usize) as u64;
        let old_brk = page_align_up(current as usize) as u64;

        if new_brk > old_brk {
            // Expand: allocate and map new pages.
            let hhdm_offset = crate::mm::hhdm::offset();
            let pages_needed = ((new_brk - old_brk) / PAGE_SIZE as u64) as usize;

            let result = pmm::with(|pmm| {
                let mut alloc = BitmapFrameAllocRef(pmm);
                for i in 0..pages_needed {
                    let page_vaddr = old_brk + (i as u64) * PAGE_SIZE as u64;
                    let frame = match alloc.0.allocate_frame() {
                        Some(f) => f,
                        None => return Err(()),
                    };

                    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                    if process
                        .address_space()
                        .map_user_page(page, frame, MapFlags::USER | MapFlags::WRITABLE, &mut alloc)
                        .is_err()
                    {
                        return Err(());
                    }

                    // Zero the page via HHDM.
                    let frame_ptr =
                        (hhdm_offset + frame.start_address().as_u64()).as_mut_ptr::<u8>();
                    // SAFETY: Frame was just allocated; zeroing via HHDM is safe.
                    unsafe {
                        core::ptr::write_bytes(frame_ptr, 0, PAGE_SIZE);
                    }
                }
                Ok(())
            });

            if result.is_err() {
                return -ENOMEM;
            }
        } else if new_brk < old_brk {
            // Shrink: unmap and free pages.
            let pages_to_free = ((old_brk - new_brk) / PAGE_SIZE as u64) as usize;

            pmm::with(|pmm| {
                for i in 0..pages_to_free {
                    let page_vaddr = new_brk + (i as u64) * PAGE_SIZE as u64;
                    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                    if let Ok(frame) = process.address_space().unmap_user_page(page) {
                        // SAFETY: The frame was allocated by brk expansion and
                        // is no longer referenced by any page table entry.
                        let _ = unsafe { pmm.deallocate_frame(frame) };
                    }
                }
            });
        }

        *brk = addr;
        addr as isize
    })
}

/// `sys_mem_create_shared` — create a shared memory object.
///
/// Allocates `size` bytes of zeroed physical memory (page-aligned) and
/// returns a file descriptor referring to the `ShmObject`. Multiple
/// processes can map this fd to share the same physical pages.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_mem_create_shared(size: usize) -> isize {
    if size == 0 {
        return -EINVAL;
    }

    let shm = match crate::ipc::shm::ShmObject::new(size) {
        Some(s) => s,
        None => return -ENOMEM,
    };

    let fd = ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        fd_table.open(shm, OpenFlags::READ | OpenFlags::WRITE)
    });

    fd.as_u32() as isize
}

/// `sys_mem_map_shared` — map a shared memory object into the process address space.
///
/// `fd` is a shared memory fd from [`sys_mem_create_shared`]. `size` is the
/// mapping length (must not exceed the object's page-aligned size). `prot`
/// is a bitmask of `PROT_READ`/`PROT_WRITE`.
///
/// Returns the mapped virtual address on success, or negated errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "returning virtual address as isize; upper bit is never set for user addresses"
)]
pub(super) fn sys_mem_map_shared(fd: usize, size: usize, prot: usize) -> isize {
    use hadron_syscall::{PROT_EXEC, PROT_READ, PROT_WRITE};

    let fd = Fd::new(fd as u32);

    if size == 0 {
        return -EINVAL;
    }

    // Look up the inode from the fd table.
    let inode = ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        fd_table.get(fd).map(|f| f.inode.clone())
    });

    let Some(inode) = inode else {
        return -EBADF;
    };

    // Get the physical frame addresses from the shm object.
    let phys_addrs = match inode.shared_phys_frames() {
        Ok(addrs) => addrs,
        Err(e) => return -e.to_errno(),
    };

    let aligned_size = page_align_up(size);
    let page_count = aligned_size / PAGE_SIZE;

    if page_count > phys_addrs.len() {
        return -EINVAL;
    }

    // Build page table flags from prot.
    let mut map_flags = MapFlags::USER;
    if prot & PROT_WRITE != 0 {
        map_flags |= MapFlags::WRITABLE;
    }
    if prot & PROT_EXEC != 0 {
        map_flags |= MapFlags::EXECUTABLE;
    }
    let _ = prot & PROT_READ;

    // Allocate virtual region from the process's mmap allocator.
    let vaddr = ProcessTable::with_current(|process| {
        let mut mmap = process.mmap_alloc.lock();
        mmap.allocate(aligned_size as u64)
    });

    let base_vaddr = match vaddr {
        Some(v) => v,
        None => return -EINVAL,
    };

    // Map the shared physical frames into user address space.
    let map_result = ProcessTable::with_current(|process| {
        pmm::with(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            for i in 0..page_count {
                let page_vaddr = base_vaddr.as_u64() + (i as u64) * PAGE_SIZE as u64;
                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_vaddr));
                let frame = PhysFrame::<Size4KiB>::containing_address(phys_addrs[i]);

                if let Err(_e) = process
                    .address_space()
                    .map_user_page(page, frame, map_flags, &mut alloc)
                {
                    return Err(i);
                }
            }
            Ok(())
        })
    });

    if let Err(_partial_count) = map_result {
        return -EINVAL;
    }

    // Track as a shared mapping (frames must NOT be freed on unmap).
    ProcessTable::with_current(|process| {
        let mut mappings = process.mmap_mappings.lock();
        mappings.insert(base_vaddr.as_u64(), MappingKind::Shared { page_count });
    });

    base_vaddr.as_u64() as isize
}
