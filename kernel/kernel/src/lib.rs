//! Hadron microkernel.
//!
//! A Zircon-style object kernel providing IPC, memory management, scheduling,
//! and capabilities. All drivers, filesystems, and networking run in userspace.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![feature(allocator_api, negative_impls, never_type)]
#![warn(missing_docs)]

extern crate alloc;

// ── Logging (hadron-log) ─────────────────────────────────────────────────

/// Re-export logging macros and public API from `hadron-log`.
pub use hadron_log;
pub use hadron_log::{
    Level, flush, kdebug, kerror, kfatal, kinfo, klog, kspan, ktrace, kwarn, set_runtime_level,
};

// ── Core type re-exports (host-testable) ──────────────────────────────────

pub use hadron_core::addr;
pub use hadron_core::cell;
pub use hadron_core::cpu_local;
pub use hadron_core::id;
pub use hadron_core::paging;
pub use hadron_core::static_assert;
pub use hadron_core::sync;
pub use hadron_core::task;

// ── Crate re-exports (preserves `crate::mm`, `crate::sched` paths) ────────

#[cfg(target_os = "none")]
pub use hadron_acpi as acpi_tables;
#[cfg(target_os = "none")]
pub use hadron_mm as mm;
#[cfg(target_os = "none")]
pub use hadron_mmio as mmio;
#[cfg(target_os = "none")]
pub use hadron_objects as objects;
#[cfg(target_os = "none")]
pub use hadron_pci as pci;
#[cfg(target_os = "none")]
pub use hadron_sched as sched;
// ── Kernel-runtime modules (require target_os = "none") ───────────────────

#[cfg(target_os = "none")]
pub mod arch;

#[cfg(target_os = "none")]
pub mod boot;

#[cfg(target_os = "none")]
pub mod entry;

#[cfg(target_os = "none")]
pub mod percpu;

#[cfg(target_os = "none")]
pub mod serial_sink;

#[cfg(target_os = "none")]
pub mod time;

#[cfg(target_os = "none")]
pub mod vmm;
