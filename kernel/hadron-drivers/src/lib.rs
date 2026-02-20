//! Hardware drivers for Hadron OS.

#![cfg_attr(not(test), no_std)]

#[cfg(target_os = "none")]
extern crate alloc;

// ── Subsystem modules ───────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
pub mod ahci;
pub mod block;
pub mod display;
#[cfg(target_os = "none")]
pub mod fs;
pub mod input;
pub mod pci;
#[cfg(target_arch = "x86_64")]
pub mod virtio;
pub mod serial;

// ── Re-exports for convenience ──────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
pub use self::serial::uart16550;

/// Anchor symbol referenced by the linker script's `EXTERN()` directive
/// to force inclusion of this crate's driver registration entries.
#[cfg(target_os = "none")]
#[used]
#[unsafe(no_mangle)]
pub static __HADRON_DRIVERS_ANCHOR: u8 = 0;
