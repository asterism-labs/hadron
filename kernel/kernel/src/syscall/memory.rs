//! Memory syscall handlers: map, unmap, brk, create_shared, map_shared.

extern crate alloc;

use alloc::sync::Arc;

use hadron_core::addr::VirtAddr;
use hadron_core::paging::{Page, Size4KiB};
use hadron_mm::mapper::{MapFlags, PageMapper};
use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_objects::object::KernelObject;
use hadron_objects::vmo::Vmo;
use hadron_syscall::*;

use super::with_handle_table;
use crate::process::with_current_process;

/// Page size constant.
const PAGE_SIZE: u64 = 4096;
/// Page offset mask.
const PAGE_MASK: u64 = PAGE_SIZE - 1;

/// `SYS_MEM_MAP(addr_hint, len, prot, flags, fd)` — map anonymous memory.
///
/// Only `MAP_ANONYMOUS` mappings are supported. Returns the virtual address
/// of the mapped region, or a negative error code.
#[expect(
    clippy::cast_possible_truncation,
    reason = "page-aligned addresses fit in isize on x86_64"
)]
pub fn sys_mem_map(_addr_hint: usize, len: usize, prot: usize, flags: usize, _fd: usize) -> isize {
    if len == 0 {
        return -EINVAL;
    }
    if flags & MAP_ANONYMOUS == 0 {
        // Only anonymous mappings supported for now.
        return -ENOSYS;
    }

    // Round up to page size.
    let len_aligned = (len as u64 + PAGE_MASK) & !PAGE_MASK;

    // Convert PROT_* to MapFlags.
    let mut map_flags = MapFlags::empty();
    if prot & PROT_WRITE != 0 {
        map_flags |= MapFlags::WRITABLE;
    }
    if prot & PROT_EXEC != 0 {
        map_flags |= MapFlags::EXECUTABLE;
    }
    // USER flag is always added by the page mapper for user pages.

    // Find a free region in the process's root VMAR.
    let vaddr = match with_current_process(|proc| {
        let vmar = proc.root_vmar();
        vmar.find_free_region(len_aligned, PAGE_SIZE)
    }) {
        Some(Some(addr)) => addr,
        _ => return -ENOMEM,
    };

    // Allocate physical frames and map each page.
    let page_count = (len_aligned / PAGE_SIZE) as usize;
    let hhdm_offset = hadron_mm::hhdm::offset();
    let mapper = crate::arch::x86_64::paging::PageTableMapper::new(hhdm_offset);
    let cr3 = crate::arch::x86_64::registers::control::Cr3::read();

    for i in 0..page_count {
        let page_addr = vaddr + (i as u64) * PAGE_SIZE;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_addr));
        let frame = hadron_mm::pmm::with(|pmm| {
            pmm.allocate_frame()
                .expect("PMM: out of frames for mem_map")
        });

        // Zero the page via HHDM.
        let hhdm_ptr = hadron_mm::hhdm::phys_to_virt(frame.start_address());
        // SAFETY: Frame was just allocated and is accessible via HHDM.
        let page_slice = unsafe {
            core::slice::from_raw_parts_mut(hhdm_ptr.as_u64() as *mut u8, PAGE_SIZE as usize)
        };
        page_slice.fill(0);

        // Map the page into the current address space with USER flag.
        let flags_with_user = map_flags | MapFlags::USER;
        hadron_mm::pmm::with(|pmm| {
            let mut alloc_fn = || {
                pmm.allocate_frame()
                    .expect("PMM: out of frames for page table")
            };
            // SAFETY: cr3 is the current process's valid PML4. The frame
            // was just allocated and zeroed.
            let flush = unsafe { mapper.map(cr3, page, frame, flags_with_user, &mut alloc_fn) };
            flush.flush();
        });
    }

    // Record the mapping in the VMAR for bookkeeping.
    let vmo = hadron_objects::vmo::Vmo::new_paged(len_aligned);
    let _ = with_current_process(|proc| {
        let vmar = proc.root_vmar();
        let vmar_flags = prot_to_vmar_flags(prot);
        let _ = vmar.map(vmo, 0, vaddr, len_aligned, vmar_flags);
    });

    vaddr as isize
}

/// `SYS_MEM_UNMAP(addr, len)` — unmap a previously mapped region.
pub fn sys_mem_unmap(addr: usize, len: usize) -> isize {
    let addr = addr as u64;
    if addr & PAGE_MASK != 0 || len == 0 {
        return -EINVAL;
    }

    let len_aligned = ((len as u64) + PAGE_MASK) & !PAGE_MASK;

    // Remove mapping from VMAR bookkeeping.
    let unmap_result = with_current_process(|proc| {
        let vmar = proc.root_vmar();
        vmar.unmap(addr, len_aligned)
    });

    match unmap_result {
        Some(Ok(())) => {}
        _ => return -EINVAL,
    }

    // Unmap each page from the page table and free the frame.
    let hhdm_offset = hadron_mm::hhdm::offset();
    let mapper = crate::arch::x86_64::paging::PageTableMapper::new(hhdm_offset);
    let cr3 = crate::arch::x86_64::registers::control::Cr3::read();
    let page_count = (len_aligned / PAGE_SIZE) as usize;

    for i in 0..page_count {
        let page_addr = addr + (i as u64) * PAGE_SIZE;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_addr));

        // SAFETY: cr3 is valid; page was previously mapped by sys_mem_map.
        let result = unsafe { mapper.unmap(cr3, page) };
        if let Ok((frame, flush)) = result {
            flush.flush();
            hadron_mm::pmm::with(|pmm| {
                // SAFETY: Frame was mapped by us and is now unmapped.
                unsafe {
                    let _ = pmm.deallocate_frame(frame);
                }
            });
        }
    }

    0
}

/// `SYS_MEM_BRK` — stub (not yet implemented).
pub fn sys_mem_brk(_addr: usize) -> isize {
    -ENOSYS
}

/// `SYS_MEM_CREATE_SHARED(size)` — create a shared memory object (VMO) backed
/// by committed physical pages.
///
/// Returns a handle to the new VMO, or a negative error code.
#[expect(
    clippy::cast_possible_truncation,
    reason = "page-aligned handle values fit in isize on x86_64"
)]
pub fn sys_mem_create_shared(size: usize) -> isize {
    if size == 0 {
        return -EINVAL;
    }

    let aligned_size = ((size as u64) + PAGE_MASK) & !PAGE_MASK;
    let vmo = Vmo::new_paged(aligned_size);
    let page_count = vmo.page_count();
    let hhdm_offset = hadron_mm::hhdm::offset();

    // Allocate and zero physical frames, committing each into the VMO.
    for i in 0..page_count {
        let frame = hadron_mm::pmm::with(|pmm| pmm.allocate_frame());
        let frame = match frame {
            Some(f) => f,
            None => return -ENOMEM,
        };

        // Zero the page via HHDM.
        let hhdm_virt = hhdm_offset + frame.start_address().as_u64();
        // SAFETY: Frame was just allocated and is accessible via HHDM.
        let page_slice = unsafe {
            core::slice::from_raw_parts_mut(hhdm_virt.as_u64() as *mut u8, PAGE_SIZE as usize)
        };
        page_slice.fill(0);

        // commit_page cannot fail here — index is always in range.
        let _ = vmo.commit_page(i, frame);
    }

    // Insert into the handle table with VMO default rights.
    let entry = HandleEntry::new(vmo as Arc<dyn KernelObject>, Rights::VMO_DEFAULT);
    with_handle_table(|table| match table.insert(entry) {
        Ok(hv) => hv.raw() as isize,
        Err(_) => -EMFILE,
    })
}

/// `SYS_MEM_MAP_SHARED(fd, size, prot)` — map an existing VMO's pages into the
/// current process address space.
///
/// Unlike `sys_mem_map` (which allocates fresh frames), this reuses the VMO's
/// committed physical frames, enabling shared memory between processes.
///
/// Returns the virtual address of the mapped region, or a negative error code.
#[expect(
    clippy::cast_possible_truncation,
    reason = "page-aligned addresses fit in isize on x86_64"
)]
pub fn sys_mem_map_shared(fd: usize, size: usize, prot: usize) -> isize {
    if size == 0 {
        return -EINVAL;
    }

    let hv = HandleValue::from_raw(fd as u32);

    // Look up the VMO handle.
    let vmo: Arc<Vmo> = match with_handle_table(|table| {
        let entry = table.get_with_rights(hv, Rights::MAP)?;
        entry
            .object()
            .as_any()
            .downcast_ref::<Vmo>()
            .map(|_| {
                // SAFETY: We verified the downcast succeeds; clone the Arc.
                // Re-borrow via the KernelObject Arc and downcast again to get
                // an owned Arc<Vmo>.
                let obj = entry.object().clone();
                // SAFETY: downcast_ref succeeded above, so this is a Vmo.
                unsafe { Arc::from_raw(Arc::into_raw(obj).cast::<Vmo>()) }
            })
            .ok_or(hadron_objects::handle::HandleError::NotFound)
    }) {
        Ok(vmo) => vmo,
        Err(_) => return -EBADF,
    };

    let len_aligned = ((size as u64) + PAGE_MASK) & !PAGE_MASK;
    let map_page_count = (len_aligned / PAGE_SIZE) as usize;
    let vmo_page_count = vmo.page_count();

    if map_page_count > vmo_page_count {
        return -EINVAL;
    }

    // Find a free region in the process's root VMAR.
    let vaddr = match with_current_process(|proc| {
        let vmar = proc.root_vmar();
        vmar.find_free_region(len_aligned, PAGE_SIZE)
    }) {
        Some(Some(addr)) => addr,
        _ => return -ENOMEM,
    };

    // Build MapFlags from prot.
    let mut map_flags = MapFlags::empty();
    if prot & PROT_WRITE != 0 {
        map_flags |= MapFlags::WRITABLE;
    }
    if prot & PROT_EXEC != 0 {
        map_flags |= MapFlags::EXECUTABLE;
    }
    let flags_with_user = map_flags | MapFlags::USER;

    let hhdm_offset = hadron_mm::hhdm::offset();
    let mapper = crate::arch::x86_64::paging::PageTableMapper::new(hhdm_offset);
    let cr3 = crate::arch::x86_64::registers::control::Cr3::read();

    // Map each VMO page into the process's address space.
    for i in 0..map_page_count {
        let frame = match vmo.page_at(i) {
            Some(f) => f,
            None => return -ENOMEM, // Page not committed
        };

        let page_addr = vaddr + (i as u64) * PAGE_SIZE;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_addr));

        hadron_mm::pmm::with(|pmm| {
            let mut alloc_fn = || {
                pmm.allocate_frame()
                    .expect("PMM: out of frames for page table")
            };
            // SAFETY: cr3 is the current process's valid PML4. The frame is a
            // committed VMO page.
            let flush = unsafe { mapper.map(cr3, page, frame, flags_with_user, &mut alloc_fn) };
            flush.flush();
        });
    }

    // Record the mapping in VMAR bookkeeping.
    let vmar_flags = prot_to_vmar_flags(prot);
    let _ = with_current_process(|proc| {
        let vmar = proc.root_vmar();
        let _ = vmar.map(vmo.clone(), 0, vaddr, len_aligned, vmar_flags);
    });

    vaddr as isize
}

/// Convert `PROT_*` flags to `VmarFlags`.
fn prot_to_vmar_flags(prot: usize) -> hadron_objects::vmar::VmarFlags {
    use hadron_objects::vmar::VmarFlags;
    let mut f = VmarFlags::empty();
    if prot & PROT_READ != 0 {
        f |= VmarFlags::READ;
    }
    if prot & PROT_WRITE != 0 {
        f |= VmarFlags::WRITE;
    }
    if prot & PROT_EXEC != 0 {
        f |= VmarFlags::EXECUTE;
    }
    f
}
