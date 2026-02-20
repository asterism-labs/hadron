//! Driver subsystem API traits and types.
//!
//! Defines the driver model used throughout the kernel:
//!
//! - **Layer 0** -- Resource types ([`IoPortRange`], [`MmioRegion`], [`IrqLine`]) representing
//!   exclusive hardware claims.
//! - **Layer 1** -- Base [`Driver`] trait providing identity and metadata.
//! - **Layer 2** -- Category traits ([`PlatformDriver`]) defining lifecycle and probe patterns.
//! - **Layer 3** -- Interface traits ([`SerialPort`], [`Framebuffer`]) describing what a device does.

extern crate alloc;

pub mod block;
pub mod category;
pub mod driver;
pub mod dyn_dispatch;
pub mod error;
pub mod framebuffer;
pub mod hw;
pub mod input;
pub mod pci;
pub mod registration;
pub mod resource;
pub mod serial;
pub mod services;

// Re-export all public types at the module root for ergonomic imports.
pub use block::{BlockDevice, IoError};
pub use category::PlatformDriver;
pub use driver::{Driver, DriverInfo, DriverState, DriverType};
pub use dyn_dispatch::{DynBlockDevice, DynBlockDeviceWrapper};
pub use error::DriverError;
pub use framebuffer::{Framebuffer, FramebufferInfo, PixelFormat};
pub use hw::{ClockSource, InterruptController, Timer};
pub use input::{KeyCode, KeyEvent, KeyboardDevice, MouseDevice, MouseEvent};
pub use pci::{PciAddress, PciBar, PciDeviceId, PciDeviceInfo};
pub use registration::{PciDriverEntry, PlatformDriverEntry};
#[cfg(target_os = "none")]
pub use registration::{BlockFsEntry, InitramFsEntry, VirtualFsEntry};
pub use resource::{IoPortRange, IrqLine, MmioRegion};
pub use serial::SerialPort;
pub use services::KernelServices;
