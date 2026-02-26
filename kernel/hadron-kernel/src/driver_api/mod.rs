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

// ── Re-export portable types from hadron-driver-api ──────────────────────

// Sub-modules (re-exported from the driver-api crate).
pub use hadron_driver_api::acpi_device;
pub use hadron_driver_api::block;
pub use hadron_driver_api::category;
pub use hadron_driver_api::device_path;
pub use hadron_driver_api::device_set;
pub use hadron_driver_api::driver;
pub use hadron_driver_api::dyn_dispatch;
pub use hadron_driver_api::error;
pub use hadron_driver_api::framebuffer;
pub use hadron_driver_api::hw;
pub use hadron_driver_api::input;
pub use hadron_driver_api::lifecycle;
pub use hadron_driver_api::net;
pub use hadron_driver_api::pci;
pub use hadron_driver_api::resource;
pub use hadron_driver_api::serial;

// Flat re-exports for ergonomic imports.
pub use hadron_driver_api::{
    AcpiDeviceId, AcpiDeviceInfo, AcpiMatchId, BlockDevice, ClockSource, DevicePath, DeviceSet,
    Driver, DriverError, DriverInfo, DriverState, DriverType, DynBlockDevice,
    DynBlockDeviceWrapper, DynNetDevice, DynNetDeviceWrapper, Framebuffer, FramebufferInfo,
    InterruptController, IoError, IoPortRange, IrqLine, KeyCode, KeyEvent, KeyboardDevice,
    MacAddress, ManagedDriver, MmioRegion, MouseDevice, MouseEvent, NetError, NetworkDevice,
    PciAddress, PciBar, PciDeviceId, PciDeviceInfo, PciDriverRegistration, PixelFormat,
    PlatformDriver, PlatformDriverRegistration, SerialPort, Timer, Watchdog,
};

// ── Kernel-local modules (depend on kernel internals) ────────────────────

pub mod capability;
pub mod probe_context;
pub mod registration;

pub use capability::{
    CapabilityAccess, CapabilityFlags, CapabilityToken, DmaCapability, HasCapability,
    IrqCapability, MmioCapability, PciConfigCapability, TaskSpawner, TimerCapability,
};
pub use probe_context::{PciProbeContext, PlatformProbeContext};
#[cfg(target_os = "none")]
pub use registration::{BlockFsEntry, InitramFsEntry, VirtualFsEntry};
pub use registration::{PciDriverEntry, PlatformDriverEntry};
