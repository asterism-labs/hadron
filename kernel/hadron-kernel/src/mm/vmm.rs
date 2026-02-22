//! Virtual memory manager — kernel glue.
//!
//! Re-exports the generic `Vmm<M>`, `KernelStack`, and `MmioMapping` from
//! `hadron-mm`. Adds the kernel-specific type alias (`KernelVmm`), global
//! VMM instance, and boot-time initialization.

pub use hadron_mm::vmm::*;

use crate::addr::{PhysAddr, VirtAddr};
use crate::boot::BootInfo;
use crate::mm::pmm::BitmapFrameAllocRef;
use crate::sync::SpinLock;

#[cfg(target_arch = "x86_64")]
type KernelMapper = crate::arch::x86_64::paging::PageTableMapper;
#[cfg(target_arch = "aarch64")]
type KernelMapper = crate::arch::aarch64::paging::AArch64PageMapper;

/// Type alias for the kernel VMM parameterised on the active architecture.
pub type KernelVmm = Vmm<KernelMapper>;

/// Global virtual memory manager.
static VMM: SpinLock<Option<KernelVmm>> = SpinLock::leveled("VMM", 2, None);

/// Initializes the VMM from boot info and the PMM.
///
/// Creates the `Vmm`, maps the initial heap, and stores globally.
pub fn init(boot_info: &impl BootInfo) {
    let hhdm_offset = boot_info.hhdm_offset();
    let root_phys = boot_info.page_table_root();

    // Find max physical address from memory map.
    let max_phys = boot_info
        .memory_map()
        .iter()
        .map(|r| r.start.as_u64() + r.size)
        .max()
        .unwrap_or(0);

    let mapper = KernelMapper::new(hhdm_offset);
    let vmm = KernelVmm::new(root_phys, mapper, hhdm_offset, max_phys);

    let mut global = VMM.lock();
    assert!(global.is_none(), "VMM already initialized");
    *global = Some(vmm);
}

/// Maps the initial heap pages via the VMM and PMM.
///
/// Returns `(heap_start, heap_size)`.
pub fn map_initial_heap() -> (usize, usize) {
    let mut vmm = VMM.lock();
    let vmm = vmm.as_mut().expect("VMM not initialized");

    let result = super::pmm::with_pmm(|pmm| {
        let mut alloc = BitmapFrameAllocRef(pmm);
        let (base, size) = vmm
            .map_initial_heap(&mut alloc)
            .expect("failed to map initial heap");
        (base.as_u64() as usize, size as usize)
    });
    // Log after releasing PMM lock to avoid PMM → LOGGER ordering violation.
    crate::ktrace_subsys!(
        mm,
        "VMM: mapped initial heap at {:#x}, size {:#x}",
        result.0,
        result.1
    );
    result
}

/// Grows the kernel heap by at least `min_bytes`.
///
/// Called by the heap allocator's growth callback.
pub fn grow_heap(min_bytes: usize) -> Option<(*mut u8, usize)> {
    let mut vmm = VMM.lock();
    let vmm = vmm.as_mut()?;

    let result = super::pmm::with_pmm(|pmm| {
        let mut alloc = BitmapFrameAllocRef(pmm);
        let (base, size) = vmm.grow_heap(min_bytes as u64, &mut alloc).ok()?;
        Some((base.as_mut_ptr::<u8>(), size as usize))
    });
    // Log after releasing PMM lock to avoid PMM → LOGGER ordering violation.
    if let Some((ptr, size)) = result {
        crate::ktrace_subsys!(mm, "VMM: grew heap by {:#x} bytes at {:#p}", size, ptr);
    }
    result
}

/// Executes a closure with a mutable reference to the global VMM.
pub fn with_vmm<R>(f: impl FnOnce(&mut KernelVmm) -> R) -> R {
    let mut vmm = VMM.lock();
    f(vmm.as_mut().expect("VMM not initialized"))
}

/// Maps an MMIO physical region into kernel virtual space.
///
/// Convenience wrapper that acquires both VMM and PMM locks internally.
/// Returns an [`MmioMapping`] RAII guard that unmaps the region on drop.
/// For permanent hardware mappings, call [`core::mem::forget`] on the guard.
pub fn map_mmio_region(phys: PhysAddr, size: u64) -> MmioMapping {
    let mapping = with_vmm(|vmm| {
        super::pmm::with_pmm(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            vmm.map_mmio(phys, size, &mut alloc, Some(default_mmio_cleanup))
                .expect("failed to map MMIO region")
        })
    });
    // Log after releasing PMM + VMM locks to avoid ordering violations.
    crate::ktrace_subsys!(
        mm,
        "VMM: mapped MMIO phys={:#x} size={:#x} -> virt={:#x}",
        phys.as_u64(),
        size,
        mapping.virt_base().as_u64()
    );
    mapping
}

/// Default cleanup callback for MMIO mappings.
///
/// Unmaps all pages in the region and returns the virtual address range to
/// the MMIO region allocator. Does NOT deallocate physical frames because
/// MMIO frames are device memory, not RAM.
fn default_mmio_cleanup(virt_base: VirtAddr, size: u64) {
    with_vmm(|vmm| {
        let page_size = super::PAGE_SIZE as u64;
        let page_count = size / page_size;
        for i in 0..page_count {
            let virt = virt_base + i * page_size;
            let page = crate::paging::Page::containing_address(virt);
            // Ignore errors — the page may already be unmapped.
            let _ = vmm.unmap_page(page);
        }
        let _ = vmm.dealloc_mmio_region(virt_base, size);
    });
}

/// Attempts to execute a closure with a mutable reference to the global VMM.
///
/// Returns `None` if the VMM lock is already held (avoiding deadlock in
/// fault handlers) or if the VMM has not been initialized yet.
pub fn try_with_vmm<R>(f: impl FnOnce(&mut KernelVmm) -> R) -> Option<R> {
    let mut vmm = VMM.try_lock()?;
    Some(f(vmm.as_mut()?))
}
