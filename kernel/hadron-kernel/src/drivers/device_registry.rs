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
use crate::driver_api::dyn_dispatch::DynBlockDevice;
use crate::driver_api::framebuffer::Framebuffer;
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
    /// Tracked driver instances.
    drivers: Vec<DriverEntry>,
}

impl DeviceRegistry {
    /// Creates an empty device registry.
    fn new() -> Self {
        Self {
            framebuffers: BTreeMap::new(),
            block_devices: BTreeMap::new(),
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
            crate::kinfo!("DeviceRegistry: registered framebuffer '{}'", leaf);
            device_paths.push(path);
            self.framebuffers.insert(leaf, fb);
        }

        for (path, dev) in devices.block_devices {
            let leaf = path.leaf().to_string();
            crate::kinfo!("DeviceRegistry: registered block device '{}'", leaf);
            device_paths.push(path);
            self.block_devices.insert(leaf, dev);
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
        crate::kinfo!("DeviceRegistry: registered framebuffer '{}'", name);
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
        crate::kinfo!("DeviceRegistry: registered block device '{}'", name);
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

    /// Removes a device by its leaf name from the registry.
    pub fn remove_device(&mut self, name: &str) -> bool {
        let fb = self.framebuffers.remove(name).is_some();
        let blk = self.block_devices.remove(name).is_some();
        fb || blk
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
                crate::kinfo!("DeviceRegistry: shutting down driver '{}'", entry.name);
                lifecycle.shutdown();
                entry.state = DriverState::Shutdown;
            }
        }
    }
}

/// Global device registry instance.
static DEVICE_REGISTRY: SpinLock<Option<DeviceRegistry>> = SpinLock::new(None);

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
