//! Virtual memory manager -- kernel-side glue.
//!
//! Wraps the core [`Vmm`] in a global static and provides initialization.

use hadron_core::mm::pmm::BitmapFrameAllocRef;
use hadron_core::sync::SpinLock;

use crate::boot::BootInfo;

#[cfg(target_arch = "x86_64")]
type KernelMapper = hadron_core::arch::x86_64::paging::PageTableMapper;
#[cfg(target_arch = "aarch64")]
type KernelMapper = hadron_core::arch::aarch64::paging::AArch64PageMapper;

/// Type alias for the kernel VMM parameterised on the active architecture.
pub type KernelVmm = hadron_core::mm::vmm::Vmm<KernelMapper>;

/// Global virtual memory manager.
static VMM: SpinLock<Option<KernelVmm>> = SpinLock::new(None);

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

    super::pmm::with_pmm(|pmm| {
        let mut alloc = BitmapFrameAllocRef(pmm);
        let (base, size) = vmm
            .map_initial_heap(&mut alloc)
            .expect("failed to map initial heap");
        (base.as_u64() as usize, size as usize)
    })
}

/// Grows the kernel heap by at least `min_bytes`.
///
/// Called by the heap allocator's growth callback.
pub fn grow_heap(min_bytes: usize) -> Option<(*mut u8, usize)> {
    let mut vmm = VMM.lock();
    let vmm = vmm.as_mut()?;

    super::pmm::with_pmm(|pmm| {
        let mut alloc = BitmapFrameAllocRef(pmm);
        let (base, size) = vmm.grow_heap(min_bytes as u64, &mut alloc).ok()?;
        Some((base.as_mut_ptr::<u8>(), size as usize))
    })
}

/// Executes a closure with a mutable reference to the global VMM.
pub fn with_vmm<R>(f: impl FnOnce(&mut KernelVmm) -> R) -> R {
    let mut vmm = VMM.lock();
    f(vmm.as_mut().expect("VMM not initialized"))
}

/// Maps an MMIO physical region into kernel virtual space.
///
/// Convenience wrapper that acquires both VMM and PMM locks internally.
/// Returns the virtual base address of the mapping.
pub fn map_mmio_region(
    phys: hadron_core::addr::PhysAddr,
    size: u64,
) -> hadron_core::addr::VirtAddr {
    with_vmm(|vmm| {
        super::pmm::with_pmm(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            let mapping = vmm
                .map_mmio(phys, size, &mut alloc, None)
                .expect("failed to map MMIO region");
            mapping.virt_base()
        })
    })
}

/// Attempts to execute a closure with a mutable reference to the global VMM.
///
/// Returns `None` if the VMM lock is already held (avoiding deadlock in
/// fault handlers) or if the VMM has not been initialized yet.
pub fn try_with_vmm<R>(f: impl FnOnce(&mut KernelVmm) -> R) -> Option<R> {
    let mut vmm = VMM.try_lock()?;
    Some(f(vmm.as_mut()?))
}
