//! Hadron kernel library.

#![cfg_attr(not(test), no_std)]
// QEMU-based integration test framework (kernel target only).
#![cfg_attr(all(test, target_os = "none"), no_main)]
#![cfg_attr(target_os = "none", feature(custom_test_frameworks))]
#![cfg_attr(all(test, target_os = "none"), test_runner(hadron_test::test_runner))]
#![cfg_attr(all(test, target_os = "none"), reexport_test_harness_main = "test_main")]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![feature(allocator_api, negative_impls, never_type)]
#![warn(missing_docs)]

extern crate alloc;

// ── Always-available modules (pure logic, host-testable) ─────────────────

pub mod addr;
pub mod cell;
pub mod driver_api;
pub mod paging;
pub mod static_assert;
pub mod sync;
pub mod task;

// ── Kernel-runtime modules (require target_os = "none") ──────────────────

#[cfg(target_os = "none")]
pub mod arch;
#[cfg(target_os = "none")]
pub mod backtrace;
#[cfg(target_os = "none")]
pub mod boot;
#[cfg(target_os = "none")]
pub mod bus;
#[cfg(target_os = "none")]
pub mod config;
#[cfg(target_os = "none")]
pub mod drivers;
#[cfg(target_os = "none")]
pub mod fs;
#[cfg(target_os = "none")]
pub mod ipc;
#[cfg(target_os = "none")]
pub mod log;
#[cfg(target_os = "none")]
pub mod mm;
#[cfg(target_os = "none")]
pub mod pci;
#[cfg(target_os = "none")]
pub mod percpu;
#[cfg(target_os = "none")]
pub mod proc;
#[cfg(target_os = "none")]
pub mod sched;
#[cfg(target_os = "none")]
pub mod syscall;
#[cfg(target_os = "none")]
pub mod time;

#[cfg(target_os = "none")]
pub use boot::kernel_init;
#[cfg(target_os = "none")]
pub use log::LogLevel;

/// Lightweight kernel initialization for integration tests.
///
/// Performs CPU init, HHDM, PMM, VMM, and heap initialization without
/// starting ACPI, PCI enumeration, or the async executor.
#[cfg(target_os = "none")]
pub fn test_init(boot_info: &impl boot::BootInfo) {
    crate::arch::cpu_init();
    crate::mm::hhdm::init(boot_info.hhdm_offset());
    crate::mm::pmm::init(boot_info);
    crate::mm::vmm::init(boot_info);
    crate::mm::heap::init();
}

#[cfg(all(test, target_os = "none"))]
hadron_test::test_entry_point!();
