//! Driver registration result types.
//!
//! [`DeviceSet`] collects the devices a driver creates during probe/init.
//! [`PciDriverRegistration`] and [`PlatformDriverRegistration`] bundle
//! the device set with an optional lifecycle handle.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::block::BlockDevice;
use super::device_path::DevicePath;
use super::dyn_dispatch::{
    DynBlockDevice, DynBlockDeviceWrapper, DynNetDevice, DynNetDeviceWrapper,
};
use super::framebuffer::Framebuffer;
use super::hw::Watchdog;
use super::lifecycle::ManagedDriver;
use super::net::NetworkDevice;

/// A collection of devices created by a driver during probe/init.
///
/// Drivers add their devices to this set, and the kernel processes the
/// set after probe completes to register them in the device registry.
pub struct DeviceSet {
    /// Framebuffer devices.
    pub framebuffers: Vec<(DevicePath, Arc<dyn Framebuffer>)>,
    /// Block devices (type-erased for dynamic dispatch).
    pub block_devices: Vec<(DevicePath, Box<dyn DynBlockDevice>)>,
    /// Network devices (type-erased for dynamic dispatch).
    pub net_devices: Vec<(DevicePath, Box<dyn DynNetDevice>)>,
    /// Watchdog devices.
    pub watchdogs: Vec<(DevicePath, Arc<dyn Watchdog>)>,
}

impl DeviceSet {
    /// Creates an empty device set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            framebuffers: Vec::new(),
            block_devices: Vec::new(),
            net_devices: Vec::new(),
            watchdogs: Vec::new(),
        }
    }

    /// Adds a block device to the set.
    ///
    /// The concrete `BlockDevice` is automatically wrapped in a
    /// [`DynBlockDeviceWrapper`] for type-erased storage.
    pub fn add_block_device<D: BlockDevice + 'static>(&mut self, path: DevicePath, device: D) {
        self.block_devices
            .push((path, Box::new(DynBlockDeviceWrapper(device))));
    }

    /// Adds a network device to the set.
    ///
    /// The concrete `NetworkDevice` is automatically wrapped in a
    /// [`DynNetDeviceWrapper`] for type-erased storage.
    pub fn add_net_device<D: NetworkDevice + 'static>(&mut self, path: DevicePath, device: D) {
        self.net_devices
            .push((path, Box::new(DynNetDeviceWrapper(device))));
    }

    /// Adds a framebuffer device to the set.
    pub fn add_framebuffer(&mut self, path: DevicePath, fb: Arc<dyn Framebuffer>) {
        self.framebuffers.push((path, fb));
    }

    /// Adds a watchdog device to the set.
    pub fn add_watchdog(&mut self, path: DevicePath, wd: Arc<dyn Watchdog>) {
        self.watchdogs.push((path, wd));
    }
}

impl Default for DeviceSet {
    fn default() -> Self {
        Self::new()
    }
}

/// Registration bundle returned by a PCI driver's probe function.
///
/// Contains the set of devices the driver created and an optional lifecycle
/// handle for power management.
pub struct PciDriverRegistration {
    /// Devices registered by this driver.
    pub devices: DeviceSet,
    /// Optional lifecycle handle for suspend/resume/shutdown.
    pub lifecycle: Option<Arc<dyn ManagedDriver>>,
}

/// Registration bundle returned by a platform driver's init function.
pub struct PlatformDriverRegistration {
    /// Devices registered by this driver.
    pub devices: DeviceSet,
    /// Optional lifecycle handle for suspend/resume/shutdown.
    pub lifecycle: Option<Arc<dyn ManagedDriver>>,
}
