//! Hadron microkernel.
//!
//! A Zircon-style object kernel providing IPC, memory management, scheduling,
//! and capabilities. All drivers, filesystems, and networking run in userspace.

#![cfg_attr(not(test), no_std)]
// QEMU-based integration test framework (kernel target only).
#![cfg_attr(all(test, target_os = "none"), no_main)]
#![cfg_attr(target_os = "none", feature(custom_test_frameworks))]
#![cfg_attr(all(test, target_os = "none"), test_runner(hadron_test::test_runner))]
#![cfg_attr(
    all(test, target_os = "none"),
    reexport_test_harness_main = "test_main"
)]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![feature(allocator_api, negative_impls, never_type)]
#![warn(missing_docs)]

extern crate alloc;

// ── Core type re-exports (host-testable) ──────────────────────────────────

pub use hadron_core::addr;
pub use hadron_core::cell;
pub use hadron_core::cpu_local;
pub use hadron_core::id;
pub use hadron_core::paging;
pub use hadron_core::static_assert;
pub use hadron_core::sync;
pub use hadron_core::task;

// ── Kernel-runtime modules (require target_os = "none") ───────────────────

#[cfg(target_os = "none")]
pub mod arch;

#[cfg(all(test, target_os = "none"))]
hadron_test::test_entry_point!();
