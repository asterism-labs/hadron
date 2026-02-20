//! Higher Half Direct Map (HHDM) global offset and address conversion.
//!
//! The HHDM maps all physical memory at a fixed virtual offset provided by
//! the bootloader. This module stores that offset globally so any code can
//! convert between physical and virtual addresses without threading the
//! offset through every call site.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::addr::{PhysAddr, VirtAddr};

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
pub fn init(offset: u64) {
    let prev =
        HHDM_OFFSET.compare_exchange(HHDM_UNINIT, offset, Ordering::Release, Ordering::Relaxed);
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
pub fn offset() -> u64 {
    let val = HHDM_OFFSET.load(Ordering::Acquire);
    assert!(val != HHDM_UNINIT, "HHDM: accessed before initialization");
    val
}

/// Converts a physical address to its HHDM virtual address.
#[inline]
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new_truncate(phys.as_u64() + HHDM_OFFSET.load(Ordering::Relaxed))
}

/// Converts an HHDM virtual address back to a physical address.
#[inline]
pub fn virt_to_phys(virt: VirtAddr) -> PhysAddr {
    PhysAddr::new(virt.as_u64() - HHDM_OFFSET.load(Ordering::Relaxed))
}
