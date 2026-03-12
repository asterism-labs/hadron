//! Higher Half Direct Map (HHDM) global offset and address conversion.
//!
//! The HHDM maps all physical memory at a fixed virtual offset provided by
//! the bootloader. This module stores that offset globally so any code can
//! convert between physical and virtual addresses without threading the
//! offset through every call site.
//!
//! During early boot the UEFI stub maps the first 4 GiB. The kernel extends
//! the HHDM beyond 4 GiB in `hhdm_extend` before PMM init.

use core::sync::atomic::AtomicBool;

use hadron_core::sync::atomic::{AtomicU64, Ordering};

use hadron_core::addr::{PhysAddr, VirtAddr};
use hadron_core::assert_unsafe_precondition;

/// Whether the HHDM offset has been initialized.
static HHDM_INIT: AtomicBool = AtomicBool::new(false);

/// Global HHDM offset, set once during early boot.
static HHDM_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Initializes the global HHDM offset. Must be called exactly once, early in boot.
///
/// # Panics
///
/// Panics if called more than once.
pub fn init(offset: VirtAddr) {
    HHDM_OFFSET.store(offset.as_u64(), Ordering::Release);
    let was_init = HHDM_INIT.swap(true, Ordering::Release);
    assert!(
        !was_init,
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
    assert!(
        HHDM_INIT.load(Ordering::Acquire),
        "HHDM: accessed before initialization"
    );
    VirtAddr::new_truncate(HHDM_OFFSET.load(Ordering::Acquire))
}

/// Converts a physical address to its HHDM virtual address.
#[inline]
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    debug_assert!(HHDM_INIT.load(Ordering::Relaxed), "HHDM not initialized");
    let offset = HHDM_OFFSET.load(Ordering::Relaxed);
    VirtAddr::new_truncate(phys.as_u64() + offset)
}

/// Converts an HHDM virtual address back to a physical address.
#[inline]
pub fn virt_to_phys(virt: VirtAddr) -> PhysAddr {
    debug_assert!(HHDM_INIT.load(Ordering::Relaxed), "HHDM not initialized");
    let offset = HHDM_OFFSET.load(Ordering::Relaxed);
    assert_unsafe_precondition!(
        virt.as_u64() >= offset,
        "virt_to_phys: address {:#x} is below HHDM base {:#x}",
        virt.as_u64(),
        offset
    );
    PhysAddr::new(virt.as_u64() - offset)
}
