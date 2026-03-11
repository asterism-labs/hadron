//! Global VMM wrapper.
//!
//! Stores the concrete `Vmm<PageTableMapper>` behind a spinlock so subsystems
//! can access it via [`with()`]. The VMM is initialized once during boot and
//! never replaced.

use hadron_core::sync::SpinLock;
use hadron_mm::vmm::Vmm;

use crate::arch::x86_64::paging::PageTableMapper;

/// Global kernel VMM instance.
///
/// Lock level 4: above PMM (3), below subsystem locks that call into VMM.
static VMM: SpinLock<Option<Vmm<PageTableMapper>>> = SpinLock::leveled("VMM", 4, None);

/// Stores the initialized VMM globally.
///
/// # Panics
///
/// Panics if called more than once.
pub fn init(vmm: Vmm<PageTableMapper>) {
    let mut guard = VMM.lock();
    assert!(guard.is_none(), "VMM already initialized");
    *guard = Some(vmm);
}

/// Executes a closure with an exclusive reference to the global VMM.
///
/// # Panics
///
/// Panics if the VMM has not been initialized.
pub fn with<R>(f: impl FnOnce(&mut Vmm<PageTableMapper>) -> R) -> R {
    let mut guard = VMM.lock();
    f(guard.as_mut().expect("VMM not initialized"))
}

/// Returns the boot CR3 physical address (the kernel's root page table).
///
/// Used by process cleanup to restore CR3 before dropping per-process
/// address spaces.
///
/// # Panics
///
/// Panics if the VMM has not been initialized.
pub fn boot_cr3() -> hadron_core::addr::PhysAddr {
    let guard = VMM.lock();
    guard.as_ref().expect("VMM not initialized").root_phys()
}
