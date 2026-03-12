//! HHDM extension for physical memory beyond 4 GiB.
//!
//! The UEFI boot stub maps the first 4 GiB of physical memory into the HHDM.
//! Systems with more than 4 GiB of RAM need the mapping extended before PMM
//! init, since the PMM bitmap may be placed above 4 GiB. This module provides
//! [`extend_hhdm`], called between memory map conversion and PMM init.

use hadron_core::addr::{PhysAddr, VirtAddr};
use hadron_core::paging::{PhysFrame, Size4KiB};

use crate::boot::BootInfo;
use hadron_mm::PhysMemoryRegion;

/// 4 GiB boundary — the boot stub maps everything below this.
const FOUR_GIB: u64 = 0x1_0000_0000;

/// 2 MiB page size.
const SIZE_2MIB: u64 = 0x20_0000;

/// 4 KiB page size.
const PAGE_SIZE: u64 = 4096;

/// Bump allocator over unused pages in the boot page table pool.
///
/// The boot stub allocates from the pool's base upward. This allocator
/// takes from the end backward, avoiding collisions.
struct BootFrameAllocator {
    /// Next page to allocate (grows downward).
    next: u64,
    /// Lower bound — the end of pages used by the boot stub.
    limit: u64,
}

impl BootFrameAllocator {
    /// Creates a new allocator over the unused portion of the boot PT pool.
    fn new(pool_phys: u64, used_pages: u64, total_pages: u64) -> Self {
        Self {
            next: pool_phys + total_pages * PAGE_SIZE,
            limit: pool_phys + used_pages * PAGE_SIZE,
        }
    }

    /// Allocate a single 4 KiB frame, zeroed. Returns the physical frame.
    ///
    /// # Panics
    ///
    /// Panics if the pool is exhausted.
    fn alloc_frame(&mut self, hhdm_offset: VirtAddr) -> PhysFrame<Size4KiB> {
        assert!(
            self.next - PAGE_SIZE >= self.limit,
            "HHDM extend: boot PT pool exhausted"
        );
        self.next -= PAGE_SIZE;
        let phys = PhysAddr::new(self.next);

        // Zero the page via the HHDM (it's within the initial 4 GiB mapping
        // since the boot stub allocated the pool from UEFI memory).
        let virt_ptr = (hhdm_offset + phys.as_u64()).as_mut_ptr::<u8>();
        // SAFETY: The pool pages are in HHDM-mapped memory below 4 GiB,
        // allocated by the boot stub. We own this page exclusively.
        unsafe { core::ptr::write_bytes(virt_ptr, 0, PAGE_SIZE as usize) };

        PhysFrame::containing_address(phys)
    }
}

/// Extend the HHDM to cover all physical memory above 4 GiB.
///
/// Must be called after the memory map is converted (step 5) and before PMM
/// init (step 7). Uses unused pages from the boot page table pool for
/// intermediate page table structures.
///
/// If `max_phys` is at or below 4 GiB, this is a no-op.
pub fn extend_hhdm(regions: &[PhysMemoryRegion], hhdm_offset: VirtAddr, bi: &BootInfo) {
    // Find the highest physical address from the memory map.
    let max_phys = regions
        .iter()
        .map(|r| r.start.as_u64() + r.size)
        .max()
        .unwrap_or(0);

    if max_phys <= FOUR_GIB {
        crate::kdebug!(
            "boot",
            "HHDM extension skipped (max_phys {:#x} <= 4 GiB)",
            max_phys
        );
        return;
    }

    crate::kinfo!(
        "boot",
        "extending HHDM: 4 GiB..{:#x} ({} MiB)",
        max_phys,
        (max_phys - FOUR_GIB) / (1024 * 1024)
    );

    // Read CR3 to get the PML4 physical address.
    let pml4_phys = crate::arch::x86_64::registers::control::Cr3::read();

    // Create mapper and frame allocator.
    let mapper = crate::arch::x86_64::paging::PageTableMapper::new(hhdm_offset);
    let mut alloc = BootFrameAllocator::new(
        bi.boot_pt_pool_phys,
        bi.boot_pt_pool_pages,
        bi.boot_pt_pool_total,
    );

    // Map physical memory from 4 GiB to max_phys as 2 MiB huge pages.
    let flags = crate::arch::x86_64::structures::paging::PageTableFlags::PRESENT
        | crate::arch::x86_64::structures::paging::PageTableFlags::WRITABLE
        | crate::arch::x86_64::structures::paging::PageTableFlags::NO_EXECUTE;

    let start = FOUR_GIB;
    let end = (max_phys + SIZE_2MIB - 1) & !(SIZE_2MIB - 1); // round up to 2 MiB

    let mut addr = start;
    while addr < end {
        let virt = VirtAddr::new_truncate(hhdm_offset.as_u64() + addr);
        let phys = PhysAddr::new(addr);

        // SAFETY: pml4_phys is the current CR3, virt is in the HHDM range,
        // phys is 2 MiB aligned, and no conflicting mapping exists above 4 GiB.
        unsafe {
            mapper.map_2mib(pml4_phys, virt, phys, flags, &mut || {
                alloc.alloc_frame(hhdm_offset)
            });
        }

        addr += SIZE_2MIB;
    }

    let pages_mapped = (end - start) / SIZE_2MIB;
    crate::kinfo!(
        "boot",
        "HHDM extended: {} 2MiB pages mapped ({} MiB)",
        pages_mapped,
        pages_mapped * 2
    );
}
