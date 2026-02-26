//! Driver API types and traits for Hadron OS.
//!
//! This crate defines the portable, host-testable driver interface types:
//!
//! - **Layer 0** — Resource types ([`IoPortRange`], [`MmioRegion`], [`IrqLine`]) representing
//!   exclusive hardware claims.
//! - **Layer 1** — Base [`Driver`] trait providing identity and metadata.
//! - **Layer 2** — Category traits ([`PlatformDriver`]) defining lifecycle and probe patterns.
//! - **Layer 3** — Interface traits ([`SerialPort`], [`Framebuffer`]) describing what a device does.
//! - **Registration** — [`DeviceSet`] and registration bundles for probe results.
//! - **Lifecycle** — [`ManagedDriver`] trait for orderly suspend/resume/shutdown.
//!
//! Capability tokens, probe contexts, and linker-section registration entries
//! remain in `hadron-kernel` because they depend on kernel internals.

#![cfg_attr(not(test), no_std)]
#![feature(negative_impls)]
#![warn(missing_docs)]

extern crate alloc;

pub mod acpi_device;
pub mod block;
pub mod category;
pub mod device_path;
pub mod device_set;
pub mod driver;
pub mod dyn_dispatch;
pub mod error;
pub mod framebuffer;
pub mod hw;
pub mod input;
pub mod lifecycle;
pub mod net;
pub mod pci;
pub mod resource;
pub mod serial;

// Re-export all public types at the crate root for ergonomic imports.
pub use acpi_device::{AcpiDeviceId, AcpiDeviceInfo, AcpiMatchId};
pub use block::{BlockDevice, IoError};
pub use category::PlatformDriver;
pub use device_path::DevicePath;
pub use device_set::{DeviceSet, PciDriverRegistration, PlatformDriverRegistration};
pub use driver::{Driver, DriverInfo, DriverState, DriverType};
pub use dyn_dispatch::{DynBlockDevice, DynBlockDeviceWrapper, DynNetDevice, DynNetDeviceWrapper};
pub use error::DriverError;
pub use framebuffer::{Framebuffer, FramebufferInfo, PixelFormat};
pub use hw::{ClockSource, InterruptController, Timer, Watchdog};
pub use input::{KeyCode, KeyEvent, KeyboardDevice, MouseDevice, MouseEvent};
pub use lifecycle::ManagedDriver;
pub use net::{MacAddress, NetError, NetworkDevice};
pub use pci::{PciAddress, PciBar, PciDeviceId, PciDeviceInfo};
pub use resource::{IoPortRange, IrqLine, MmioRegion};
pub use serial::SerialPort;
