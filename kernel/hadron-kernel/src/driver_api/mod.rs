//! Driver subsystem API traits and types.
//!
//! Defines the driver model used throughout the kernel:
//!
//! - **Layer 0** -- Resource types ([`IoPortRange`], [`MmioRegion`], [`IrqLine`]) representing
//!   exclusive hardware claims.
//! - **Layer 1** -- Base [`Driver`] trait providing identity and metadata.
//! - **Layer 2** -- Category traits ([`PlatformDriver`]) defining lifecycle and probe patterns.
//! - **Layer 3** -- Interface traits ([`SerialPort`], [`Framebuffer`]) describing what a device does.
//! - **Capabilities** -- Typed tokens ([`IrqCapability`], [`MmioCapability`], etc.) providing
//!   scoped access to kernel subsystems.
//! - **Probe Contexts** -- [`PciProbeContext`] and [`PlatformProbeContext`] bundle capabilities
//!   for driver initialization.
//! - **Lifecycle** -- [`ManagedDriver`] trait for orderly suspend/resume/shutdown.

extern crate alloc;

pub mod acpi_device;
pub mod block;
pub mod capability;
pub mod category;
pub mod device_path;
pub mod driver;
pub mod dyn_dispatch;
pub mod error;
pub mod framebuffer;
pub mod hw;
pub mod input;
pub mod lifecycle;
pub mod net;
pub mod pci;
pub mod probe_context;
pub mod registration;
pub mod resource;
pub mod serial;

// Re-export all public types at the module root for ergonomic imports.
pub use acpi_device::{AcpiDeviceId, AcpiDeviceInfo, AcpiMatchId};
pub use block::{BlockDevice, IoError};
pub use capability::{
    CapabilityAccess, CapabilityFlags, CapabilityToken, DmaCapability, HasCapability,
    IrqCapability, MmioCapability, PciConfigCapability, TaskSpawner, TimerCapability,
};
pub use category::PlatformDriver;
pub use device_path::DevicePath;
pub use driver::{Driver, DriverInfo, DriverState, DriverType};
pub use dyn_dispatch::{DynBlockDevice, DynBlockDeviceWrapper, DynNetDevice, DynNetDeviceWrapper};
pub use error::DriverError;
pub use framebuffer::{Framebuffer, FramebufferInfo, PixelFormat};
pub use hw::{ClockSource, InterruptController, Timer};
pub use input::{KeyCode, KeyEvent, KeyboardDevice, MouseDevice, MouseEvent};
pub use lifecycle::ManagedDriver;
pub use net::{MacAddress, NetError, NetworkDevice};
pub use pci::{PciAddress, PciBar, PciDeviceId, PciDeviceInfo};
pub use probe_context::{PciProbeContext, PlatformProbeContext};
#[cfg(target_os = "none")]
pub use registration::{BlockFsEntry, InitramFsEntry, VirtualFsEntry};
pub use registration::{
    DeviceSet, PciDriverEntry, PciDriverRegistration, PlatformDriverEntry,
    PlatformDriverRegistration,
};
pub use resource::{IoPortRange, IrqLine, MmioRegion};
pub use serial::SerialPort;
