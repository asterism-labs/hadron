//! Central device registry for kernel-driver decoupling.
//!
//! Drivers register their devices here during probe, and kernel subsystems
//! retrieve them by name. This breaks the direct dependency from hadron-kernel
//! on hadron-drivers: the kernel no longer needs to `use hadron_drivers::*`
//! to access specific device types.
//!
//! Device categories:
//! - **Framebuffers**: `Arc<dyn Framebuffer>` — display output
//! - **Block devices**: `Box<dyn DynBlockDevice>` — storage (take-once ownership)
//! - **Network devices**: `Box<dyn DynNetDevice>` — network I/O (take-once ownership)
//!
//! The registry also tracks driver lifecycle state via [`DriverEntry`] records,
//! enabling orderly shutdown and health inspection.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::driver_api::device_path::DevicePath;
use crate::driver_api::driver::DriverState;
use crate::driver_api::dyn_dispatch::{DynBlockDevice, DynNetDevice};
use crate::driver_api::framebuffer::Framebuffer;
use crate::driver_api::hw::Watchdog;
use crate::driver_api::lifecycle::ManagedDriver;
use crate::driver_api::registration::DeviceSet;

use crate::sync::SpinLock;

/// Per-driver tracking record.
struct DriverEntry {
    /// Driver name (from the linker section entry).
    name: String,
    /// Current lifecycle state.
    state: DriverState,
    /// Optional lifecycle handle for suspend/resume/shutdown.
    lifecycle: Option<Arc<dyn ManagedDriver>>,
    /// Device paths registered by this driver.
    device_paths: Vec<DevicePath>,
}

/// The kernel's central device registry.
///
/// Drivers register their devices during probe via [`DeviceSet`] bundles
/// returned from probe functions. Kernel subsystems retrieve devices by name.
pub struct DeviceRegistry {
    /// Named framebuffer devices.
    framebuffers: BTreeMap<String, Arc<dyn Framebuffer>>,
    /// Named block devices (take-once: removed on retrieval).
    block_devices: BTreeMap<String, Box<dyn DynBlockDevice>>,
    /// Named network devices (take-once: removed on retrieval).
    net_devices: BTreeMap<String, Box<dyn DynNetDevice>>,
    /// Named watchdog devices.
    watchdogs: BTreeMap<String, Arc<dyn Watchdog>>,
    /// Tracked driver instances.
    drivers: Vec<DriverEntry>,
}

impl DeviceRegistry {
    /// Creates an empty device registry.
    fn new() -> Self {
        Self {
            framebuffers: BTreeMap::new(),
            block_devices: BTreeMap::new(),
            net_devices: BTreeMap::new(),
            watchdogs: BTreeMap::new(),
            drivers: Vec::new(),
        }
    }

    /// Registers a driver and its device set from a probe result.
    ///
    /// Processes the [`DeviceSet`] to add individual devices to the registry,
    /// and records the driver's lifecycle handle for power management.
    pub fn register_driver(
        &mut self,
        name: &str,
        devices: DeviceSet,
        lifecycle: Option<Arc<dyn ManagedDriver>>,
    ) {
        let mut device_paths = Vec::new();

        for (path, fb) in devices.framebuffers {
            let leaf = path.leaf().to_string();
            device_paths.push(path);
            self.framebuffers.insert(leaf, fb);
        }

        for (path, dev) in devices.block_devices {
            let leaf = path.leaf().to_string();
            device_paths.push(path);
            self.block_devices.insert(leaf, dev);
        }

        for (path, dev) in devices.net_devices {
            let leaf = path.leaf().to_string();
            device_paths.push(path);
            self.net_devices.insert(leaf, dev);
        }

        for (path, wd) in devices.watchdogs {
            let leaf = path.leaf().to_string();
            device_paths.push(path);
            self.watchdogs.insert(leaf, wd);
        }

        self.drivers.push(DriverEntry {
            name: name.to_string(),
            state: DriverState::Active,
            lifecycle,
            device_paths,
        });
    }

    /// Registers a framebuffer device directly (backward-compat).
    pub fn register_framebuffer(&mut self, name: &str, fb: Arc<dyn Framebuffer>) {
        self.framebuffers.insert(name.to_string(), fb);
    }

    /// Returns a reference to a named framebuffer, if registered.
    pub fn framebuffer(&self, name: &str) -> Option<&Arc<dyn Framebuffer>> {
        self.framebuffers.get(name)
    }

    /// Returns a clone of a named framebuffer Arc, if registered.
    pub fn take_framebuffer(&self, name: &str) -> Option<Arc<dyn Framebuffer>> {
        self.framebuffers.get(name).cloned()
    }

    /// Registers a block device directly (backward-compat).
    pub fn register_block_device(&mut self, name: &str, dev: Box<dyn DynBlockDevice>) {
        self.block_devices.insert(name.to_string(), dev);
    }

    /// Takes ownership of a named block device, removing it from the registry.
    ///
    /// Returns `None` if the device was not registered or was already taken.
    pub fn take_block_device(&mut self, name: &str) -> Option<Box<dyn DynBlockDevice>> {
        self.block_devices.remove(name)
    }

    /// Returns the number of registered block devices.
    pub fn block_device_count(&self) -> usize {
        self.block_devices.len()
    }

    /// Returns an iterator over registered block device names.
    pub fn block_device_names(&self) -> impl Iterator<Item = &str> {
        self.block_devices.keys().map(String::as_str)
    }

    /// Takes ownership of a named network device, removing it from the registry.
    ///
    /// Returns `None` if the device was not registered or was already taken.
    pub fn take_net_device(&mut self, name: &str) -> Option<Box<dyn DynNetDevice>> {
        self.net_devices.remove(name)
    }

    /// Returns the number of registered network devices.
    pub fn net_device_count(&self) -> usize {
        self.net_devices.len()
    }

    /// Returns an iterator over registered network device names.
    pub fn net_device_names(&self) -> impl Iterator<Item = &str> {
        self.net_devices.keys().map(String::as_str)
    }

    /// Returns the first registered watchdog device, if any.
    pub fn first_watchdog(&self) -> Option<Arc<dyn Watchdog>> {
        self.watchdogs.values().next().cloned()
    }

    /// Removes a device by its leaf name from the registry.
    pub fn remove_device(&mut self, name: &str) -> bool {
        let fb = self.framebuffers.remove(name).is_some();
        let blk = self.block_devices.remove(name).is_some();
        let net = self.net_devices.remove(name).is_some();
        let wd = self.watchdogs.remove(name).is_some();
        fb || blk || net || wd
    }

    /// Performs orderly shutdown of all registered drivers.
    ///
    /// Calls [`ManagedDriver::shutdown`] on each driver that has a lifecycle
    /// handle, in reverse registration order.
    pub fn shutdown_all(&mut self) {
        for entry in self.drivers.iter_mut().rev() {
            if entry.state != DriverState::Active && entry.state != DriverState::Suspended {
                continue;
            }
            if let Some(ref lifecycle) = entry.lifecycle {
                lifecycle.shutdown();
                entry.state = DriverState::Shutdown;
            }
        }
    }
}

/// Global device registry instance.
static DEVICE_REGISTRY: SpinLock<Option<DeviceRegistry>> =
    SpinLock::leveled("DEVICE_REGISTRY", 4, None);

/// Initializes the global device registry.
///
/// Must be called before any driver probing begins.
pub fn init() {
    let mut guard = DEVICE_REGISTRY.lock();
    *guard = Some(DeviceRegistry::new());
}

/// Executes a closure with a shared reference to the device registry.
///
/// # Panics
///
/// Panics if the device registry has not been initialized via [`init`].
pub fn with_device_registry<R>(f: impl FnOnce(&DeviceRegistry) -> R) -> R {
    let guard = DEVICE_REGISTRY.lock();
    let registry = guard
        .as_ref()
        .expect("device registry not initialized — call device_registry::init() first");
    f(registry)
}

/// Executes a closure with a mutable reference to the device registry.
///
/// # Panics
///
/// Panics if the device registry has not been initialized via [`init`].
pub fn with_device_registry_mut<R>(f: impl FnOnce(&mut DeviceRegistry) -> R) -> R {
    let mut guard = DEVICE_REGISTRY.lock();
    let registry = guard
        .as_mut()
        .expect("device registry not initialized — call device_registry::init() first");
    f(registry)
}
