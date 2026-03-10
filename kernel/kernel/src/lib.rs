//! Hadron microkernel.
//!
//! A Zircon-style object kernel providing IPC, memory management, scheduling,
//! and capabilities. All drivers, filesystems, and networking run in userspace.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![feature(allocator_api, negative_impls, never_type)]
#![warn(missing_docs)]

extern crate alloc;

// ── Logging stubs (replaced by serial output in a later phase) ──────────

/// Log an informational message (no-op stub).
#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => {};
}

/// Log a debug message (no-op stub).
#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => {};
}

/// Log a warning message (no-op stub).
#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => {};
}

/// Log an error message (no-op stub).
#[macro_export]
macro_rules! kerr {
    ($($arg:tt)*) => {};
}

/// Subsystem-level trace logging (no-op stub).
#[macro_export]
macro_rules! ktrace_subsys {
    ($subsys:ident, $($arg:tt)*) => {};
}

// ── Core type re-exports (host-testable) ──────────────────────────────────

pub use hadron_core::addr;
pub use hadron_core::cell;
pub use hadron_core::cpu_local;
pub use hadron_core::id;
pub use hadron_core::paging;
pub use hadron_core::static_assert;
pub use hadron_core::sync;
pub use hadron_core::task;

// ── Crate re-exports (preserves `crate::mm` paths in arch code) ───────────

#[cfg(target_os = "none")]
pub use hadron_mm as mm;

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
pub mod time;
