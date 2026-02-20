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

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;

use crate::driver_api::dyn_dispatch::DynBlockDevice;
use crate::driver_api::framebuffer::Framebuffer;

use crate::sync::SpinLock;

/// The kernel's central device registry.
///
/// Drivers register their devices during probe by calling methods on
/// [`KernelServices`](crate::driver_api::services::KernelServices).
/// Kernel subsystems retrieve devices by name.
pub struct DeviceRegistry {
    /// Named framebuffer devices.
    framebuffers: BTreeMap<String, Arc<dyn Framebuffer>>,
    /// Named block devices (take-once: removed on retrieval).
    block_devices: BTreeMap<String, Box<dyn DynBlockDevice>>,
}

impl DeviceRegistry {
    /// Creates an empty device registry.
    fn new() -> Self {
        Self {
            framebuffers: BTreeMap::new(),
            block_devices: BTreeMap::new(),
        }
    }

    /// Registers a framebuffer device.
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

    /// Registers a block device.
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
