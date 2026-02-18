//! Hadron kernel library.

#![no_std]
#![cfg_attr(test, no_main)]
#![cfg_attr(test, feature(custom_test_frameworks))]
#![cfg_attr(test, test_runner(hadron_test::test_runner))]
#![cfg_attr(test, reexport_test_harness_main = "test_main")]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![warn(missing_docs)]

extern crate alloc;

pub mod arch;
pub mod backtrace;
pub mod boot;
pub mod drivers;
pub mod fs;
pub mod log;
pub mod mm;
pub mod proc;
pub mod sched;
pub mod services;
pub mod sync;
pub mod syscall;
pub mod time;

pub use boot::kernel_init;
pub use hadron_core::log::LogLevel;
pub use hadron_core::{kdebug, kerr, kfatal, kinfo, klog, ktrace, kwarn};
pub use hadron_core::{kprint, kprintln};

/// Lightweight kernel initialization for integration tests.
///
/// Performs CPU init, HHDM, PMM, VMM, and heap initialization without
/// starting ACPI, PCI enumeration, or the async executor.
pub fn test_init(boot_info: &impl boot::BootInfo) {
    crate::arch::cpu_init();
    hadron_core::mm::hhdm::init(boot_info.hhdm_offset());
    crate::mm::pmm::init(boot_info);
    crate::mm::vmm::init(boot_info);
    crate::mm::heap::init();
}

#[cfg(test)]
hadron_test::test_entry_point!();
