//! Hardware drivers for Hadron OS.

#![cfg_attr(not(test), no_std)]

#[cfg(target_os = "none")]
extern crate alloc;

#[cfg(target_arch = "x86_64")]
pub mod ahci;
#[cfg(target_arch = "x86_64")]
pub mod apic;
#[cfg(target_arch = "x86_64")]
pub mod bochs_vga;
#[cfg(target_arch = "x86_64")]
pub mod bus;
#[cfg(target_arch = "x86_64")]
pub mod hpet;
pub mod i8042;
#[cfg(target_arch = "x86_64")]
pub mod irq;
#[cfg(target_arch = "x86_64")]
pub mod keyboard_async;
#[cfg(target_arch = "x86_64")]
pub mod mouse_async;
pub mod pci;
#[cfg(target_arch = "x86_64")]
pub mod pci_stub;
#[cfg(target_arch = "x86_64")]
pub mod pic;
#[cfg(target_arch = "x86_64")]
pub mod pit;
#[cfg(target_os = "none")]
pub mod registry;
#[cfg(target_arch = "x86_64")]
pub mod serial_async;
#[cfg(target_arch = "x86_64")]
pub mod tsc;
#[cfg(target_arch = "x86_64")]
pub mod uart16550;

/// Anchor symbol referenced by the linker script's `EXTERN()` directive
/// to force inclusion of this crate's driver registration entries.
#[cfg(target_os = "none")]
#[used]
#[unsafe(no_mangle)]
pub static __HADRON_DRIVERS_ANCHOR: u8 = 0;
