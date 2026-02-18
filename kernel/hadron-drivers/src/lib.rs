//! Hardware drivers for Hadron OS.

#![cfg_attr(not(test), no_std)]

#[cfg(target_os = "none")]
extern crate alloc;

// ── Subsystem modules ───────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
pub mod ahci;
#[cfg(target_arch = "x86_64")]
pub mod bus;
pub mod display;
pub mod input;
pub mod interrupt;
#[cfg(target_arch = "x86_64")]
pub mod irq;
pub mod pci;
#[cfg(target_os = "none")]
pub mod registry;
pub mod serial;
pub mod timer;

// ── Re-exports for backward compatibility ───────────────────────────────

#[cfg(target_arch = "x86_64")]
pub use self::display::bochs_vga;
pub use self::input::i8042;
#[cfg(target_arch = "x86_64")]
pub use self::input::keyboard_async;
#[cfg(target_arch = "x86_64")]
pub use self::input::mouse_async;
#[cfg(target_arch = "x86_64")]
pub use self::interrupt::apic;
#[cfg(target_arch = "x86_64")]
pub use self::interrupt::pic;
#[cfg(target_arch = "x86_64")]
pub use self::serial::serial_async;
#[cfg(target_arch = "x86_64")]
pub use self::serial::uart16550;
#[cfg(target_arch = "x86_64")]
pub use self::timer::{hpet, pit, tsc};

/// Anchor symbol referenced by the linker script's `EXTERN()` directive
/// to force inclusion of this crate's driver registration entries.
#[cfg(target_os = "none")]
#[used]
#[unsafe(no_mangle)]
pub static __HADRON_DRIVERS_ANCHOR: u8 = 0;
