//! Driver subsystem API traits and types for Hadron OS.
//!
//! This crate defines the driver model used throughout the kernel:
//!
//! - **Layer 0** -- Resource types ([`IoPortRange`], [`MmioRegion`], [`IrqLine`]) representing
//!   exclusive hardware claims.
//! - **Layer 1** -- Base [`Driver`] trait providing identity and metadata.
//! - **Layer 2** -- Category traits ([`PlatformDriver`]) defining lifecycle and probe patterns.
//! - **Layer 3** -- Interface traits ([`SerialPort`], [`Framebuffer`]) describing what a device does.

#![cfg_attr(not(test), no_std)]

pub mod block;
pub mod category;
pub mod driver;
pub mod error;
pub mod framebuffer;
pub mod hw;
pub mod input;
pub mod pci;
pub mod registration;
pub mod resource;
pub mod serial;
pub mod services;

// Re-export all public types at the crate root for ergonomic imports.
pub use block::{BlockDevice, IoError};
pub use category::PlatformDriver;
pub use driver::{Driver, DriverInfo, DriverState, DriverType};
pub use error::DriverError;
pub use services::KernelServices;
pub use framebuffer::{Framebuffer, FramebufferInfo, PixelFormat};
pub use pci::{PciAddress, PciBar, PciDeviceId, PciDeviceInfo};
pub use registration::{PciDriverEntry, PlatformDriverEntry};
pub use resource::{IoPortRange, IrqLine, MmioRegion};
pub use input::{KeyCode, KeyEvent, KeyboardDevice, MouseDevice, MouseEvent};
pub use serial::SerialPort;
pub use hw::{InterruptController, ClockSource, Timer};
