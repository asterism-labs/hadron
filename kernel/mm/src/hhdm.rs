//! Higher Half Direct Map (HHDM) global offset and address conversion.
//!
//! The HHDM maps all physical memory at a fixed virtual offset provided by
//! the bootloader. This module stores that offset globally so any code can
//! convert between physical and virtual addresses without threading the
//! offset through every call site.
//!
//! During early boot, a [`BootMapper`] can be registered to extend the HHDM
//! on-demand before the kernel builds its own page tables. Once the kernel
//! switches CR3, the boot mapper must be cleared.

use core::ptr::null_mut;
use core::sync::atomic::AtomicPtr;

use hadron_boot_info::{BootMapFlags, BootServices};
use hadron_core::sync::atomic::{AtomicU64, Ordering};

use hadron_core::addr::{PhysAddr, VirtAddr};
use hadron_core::assert_unsafe_precondition;

/// Sentinel value indicating the HHDM offset has not been initialized.
const HHDM_UNINIT: u64 = u64::MAX;

/// Global HHDM offset, set once during early boot.
/// Uses `u64::MAX` as an uninitialized sentinel to catch access-before-init.
static HHDM_OFFSET: AtomicU64 = AtomicU64::new(HHDM_UNINIT);

/// Initializes the global HHDM offset. Must be called exactly once, early in boot.
///
/// # Panics
///
/// Panics if called more than once.
pub fn init(offset: VirtAddr) {
    let prev = HHDM_OFFSET.compare_exchange(
        HHDM_UNINIT,
        offset.as_u64(),
        Ordering::Release,
        Ordering::Relaxed,
    );
    assert!(
        prev.is_ok(),
        "HHDM: already initialized (double init detected)"
    );
}

/// Returns the HHDM offset.
///
/// # Panics
///
/// Panics if called before [`init`].
#[inline]
pub fn offset() -> VirtAddr {
    let val = HHDM_OFFSET.load(Ordering::Acquire);
    assert!(val != HHDM_UNINIT, "HHDM: accessed before initialization");
    VirtAddr::new_truncate(val)
}

/// Converts a physical address to its HHDM virtual address.
#[inline]
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    let offset = HHDM_OFFSET.load(Ordering::Relaxed);
    debug_assert!(offset != HHDM_UNINIT, "HHDM not initialized");
    VirtAddr::new_truncate(phys.as_u64() + offset)
}

/// Converts an HHDM virtual address back to a physical address.
#[inline]
pub fn virt_to_phys(virt: VirtAddr) -> PhysAddr {
    let offset = HHDM_OFFSET.load(Ordering::Relaxed);
    assert_unsafe_precondition!(
        virt.as_u64() >= offset,
        "virt_to_phys: address {:#x} is below HHDM base {:#x}",
        virt.as_u64(),
        offset
    );
    PhysAddr::new(virt.as_u64() - offset)
}

// ── Boot mapper (early-boot on-demand HHDM extension) ────────────────

/// Boot services `map_pages` function pointer, stored atomically.
/// Non-null while the boot mapper is registered.
static BOOT_MAP_FN: AtomicPtr<()> = AtomicPtr::new(null_mut());

/// Boot services context pointer, stored atomically.
static BOOT_MAP_CTX: AtomicPtr<()> = AtomicPtr::new(null_mut());

/// Trait for early-boot page mapping before the kernel has its own page tables.
pub trait BootMapper {
    /// Ensure physical pages are mapped in the HHDM. Returns HHDM virtual address.
    fn map_pages(&self, phys: PhysAddr, count: usize, writable: bool) -> Option<VirtAddr>;
}

/// Wraps the raw `BootServices` vtable from the boot stub.
pub struct RawBootMapper {
    svc: *const BootServices,
}

impl RawBootMapper {
    /// Creates a new `RawBootMapper` from a boot services pointer.
    ///
    /// # Safety
    ///
    /// `svc` must point to a valid `BootServices` vtable that remains valid
    /// until the kernel switches CR3.
    pub unsafe fn new(svc: *const BootServices) -> Self {
        Self { svc }
    }
}

impl BootMapper for RawBootMapper {
    fn map_pages(&self, phys: PhysAddr, count: usize, writable: bool) -> Option<VirtAddr> {
        let flags = if writable {
            BootMapFlags::WRITABLE
        } else {
            BootMapFlags(0)
        };
        // SAFETY: svc points to a valid BootServices vtable; the stub's page
        // tables are still in CR3.
        let result = unsafe {
            let svc = &*self.svc;
            (svc.map_pages)(svc.ctx, phys.as_u64(), count as u64, flags)
        };
        if result != 0 {
            Some(VirtAddr::new_truncate(result))
        } else {
            None
        }
    }
}

/// Registers the boot services mapper (called once during early init).
///
/// After registration, [`ensure_mapped`] will use the boot services callback
/// to map physical pages that aren't already in the HHDM.
///
/// # Safety
///
/// `svc` must point to a valid `BootServices` vtable. The stub's page tables
/// must be in CR3.
pub unsafe fn register_boot_mapper(svc: *const BootServices) {
    if svc.is_null() {
        return;
    }
    // SAFETY: Caller guarantees svc is valid.
    let svc_ref = unsafe { &*svc };
    BOOT_MAP_CTX.store(svc_ref.ctx, Ordering::Release);
    BOOT_MAP_FN.store(svc_ref.map_pages as *mut (), Ordering::Release);
}

/// Unregisters the boot mapper (called after the kernel switches CR3).
///
/// After this call, [`ensure_mapped`] becomes a no-op.
pub fn clear_boot_mapper() {
    BOOT_MAP_FN.store(null_mut(), Ordering::Release);
    BOOT_MAP_CTX.store(null_mut(), Ordering::Release);
}

/// Ensures a physical range is accessible via the HHDM.
///
/// If a boot mapper is registered, calls it to map the range. Otherwise
/// this is a no-op (assumes the range is already mapped).
pub fn ensure_mapped(phys: PhysAddr, size: u64) {
    let fn_ptr = BOOT_MAP_FN.load(Ordering::Acquire);
    if fn_ptr.is_null() {
        return;
    }
    let ctx = BOOT_MAP_CTX.load(Ordering::Acquire);
    let page_size = 0x1000u64;
    let count = (size + page_size - 1) / page_size;

    // SAFETY: fn_ptr and ctx were set by register_boot_mapper from a valid
    // BootServices. The stub's page tables are still active.
    let map_fn: unsafe extern "C" fn(*mut (), u64, u64, BootMapFlags) -> u64 =
        unsafe { core::mem::transmute(fn_ptr) };
    unsafe {
        map_fn(ctx, phys.as_u64(), count, BootMapFlags::WRITABLE);
    }
}
