//! Linker-section-based driver registration types and macros.
//!
//! Driver crates use the `#[hadron_driver(...)]` attribute macro to declare
//! PCI and platform drivers. Filesystem drivers use [`block_fs_entry!`],
//! [`virtual_fs_entry!`], and [`initramfs_entry!`] to place static entries
//! into dedicated linker sections. The kernel iterates these sections at boot
//! to discover and match drivers to devices, and mount filesystems — no
//! runtime registry needed.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use super::block::BlockDevice;
use super::device_path::DevicePath;
use super::dyn_dispatch::{DynBlockDevice, DynBlockDeviceWrapper};
use super::error::DriverError;
use super::framebuffer::Framebuffer;
use super::lifecycle::ManagedDriver;
use super::pci::PciDeviceId;
use super::probe_context::{PciProbeContext, PlatformProbeContext};

// ---------------------------------------------------------------------------
// PCI driver entry
// ---------------------------------------------------------------------------

/// PCI driver entry placed in the `.hadron_pci_drivers` linker section.
///
/// Contains the driver's name, supported device IDs, declared capabilities,
/// and a probe function. The kernel iterates these entries to match
/// discovered PCI devices to drivers.
#[repr(C)]
pub struct PciDriverEntry {
    /// Driver name (for logging).
    pub name: &'static str,
    /// Device IDs this driver supports.
    pub id_table: &'static [PciDeviceId],
    /// Declared capability flags for runtime auditing.
    pub capabilities: super::capability::CapabilityFlags,
    /// Called when a matching device is found.
    ///
    /// Receives a [`PciProbeContext`] with typed capability tokens.
    /// Returns a [`PciDriverRegistration`] describing the devices created.
    pub probe: fn(PciProbeContext) -> Result<PciDriverRegistration, DriverError>,
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

// ---------------------------------------------------------------------------
// Platform driver entry
// ---------------------------------------------------------------------------

/// Platform driver entry placed in the `.hadron_platform_drivers` linker section.
///
/// Platform drivers are matched by ACPI `_HID`/`_CID` values using an
/// [`AcpiMatchId`] table.
#[repr(C)]
pub struct PlatformDriverEntry {
    /// Driver name (for logging).
    pub name: &'static str,
    /// ACPI ID table for matching against device `_HID`/`_CID`.
    pub id_table: &'static [super::acpi_device::AcpiMatchId],
    /// Declared capability flags for runtime auditing.
    pub capabilities: super::capability::CapabilityFlags,
    /// Probe function called when a matching device is found.
    ///
    /// Receives a [`PlatformProbeContext`] with typed capability tokens.
    /// Returns a [`PlatformDriverRegistration`] describing the devices created.
    pub probe: fn(PlatformProbeContext) -> Result<PlatformDriverRegistration, DriverError>,
}

/// Registration bundle returned by a platform driver's init function.
pub struct PlatformDriverRegistration {
    /// Devices registered by this driver.
    pub devices: DeviceSet,
    /// Optional lifecycle handle for suspend/resume/shutdown.
    pub lifecycle: Option<Arc<dyn ManagedDriver>>,
}

// ---------------------------------------------------------------------------
// DeviceSet — bundle of devices a driver registers
// ---------------------------------------------------------------------------

/// A collection of devices created by a driver during probe/init.
///
/// Drivers add their devices to this set, and the kernel processes the
/// set after probe completes to register them in the device registry.
pub struct DeviceSet {
    /// Framebuffer devices.
    pub(crate) framebuffers: Vec<(DevicePath, Arc<dyn Framebuffer>)>,
    /// Block devices (type-erased for dynamic dispatch).
    pub(crate) block_devices: Vec<(DevicePath, Box<dyn DynBlockDevice>)>,
}

impl DeviceSet {
    /// Creates an empty device set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            framebuffers: Vec::new(),
            block_devices: Vec::new(),
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

    /// Adds a framebuffer device to the set.
    pub fn add_framebuffer(&mut self, path: DevicePath, fb: Arc<dyn Framebuffer>) {
        self.framebuffers.push((path, fb));
    }
}

impl Default for DeviceSet {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Filesystem entries (unchanged from previous design)
// ---------------------------------------------------------------------------

/// Block-device filesystem entry placed in the `.hadron_block_fs` linker section.
///
/// The kernel iterates these entries at boot to mount block-device-backed
/// filesystems (e.g., FAT, ISO 9660). The `mount` function receives a
/// type-erased block device and is responsible for creating a
/// [`BlockDeviceAdapter`](crate::fs::block_adapter::BlockDeviceAdapter) internally.
#[cfg(target_os = "none")]
#[repr(C)]
pub struct BlockFsEntry {
    /// Filesystem type name (for logging, e.g., "fat", "iso9660").
    pub name: &'static str,
    /// Mount function: takes a type-erased block device and returns a filesystem.
    pub mount: fn(
        alloc::boxed::Box<dyn crate::driver_api::dyn_dispatch::DynBlockDevice>,
    ) -> Result<Arc<dyn crate::fs::FileSystem>, crate::fs::FsError>,
}

/// Virtual filesystem entry placed in the `.hadron_virtual_fs` linker section.
///
/// Virtual filesystems (e.g., ramfs) are memory-backed and don't need a block
/// device. The `create` function instantiates a fresh filesystem.
#[cfg(target_os = "none")]
#[repr(C)]
pub struct VirtualFsEntry {
    /// Filesystem type name (for logging, e.g., "ramfs").
    pub name: &'static str,
    /// Factory function: creates a new filesystem instance.
    pub create: fn() -> Arc<dyn crate::fs::FileSystem>,
}

/// Initramfs unpacker entry placed in the `.hadron_initramfs` linker section.
///
/// The kernel calls the registered `unpack` function during boot to extract
/// the CPIO initrd archive into the root filesystem.
#[cfg(target_os = "none")]
#[repr(C)]
pub struct InitramFsEntry {
    /// Unpacker name (for logging, e.g., "cpio").
    pub name: &'static str,
    /// Unpack function: extracts archive data into the given root inode.
    /// Returns the number of files unpacked.
    pub unpack: fn(&[u8], &Arc<dyn crate::fs::Inode>) -> usize,
}

// SAFETY: These are repr(C) structs containing only references to 'static data
// and function pointers, all of which are inherently safe to share across threads.
unsafe impl Sync for PciDriverEntry {}
unsafe impl Sync for PlatformDriverEntry {}
#[cfg(target_os = "none")]
unsafe impl Sync for BlockFsEntry {}
#[cfg(target_os = "none")]
unsafe impl Sync for VirtualFsEntry {}
#[cfg(target_os = "none")]
unsafe impl Sync for InitramFsEntry {}

/// Register a block filesystem entry in the `.hadron_block_fs` linker section.
#[macro_export]
macro_rules! block_fs_entry {
    ($name:ident, $entry:expr) => {
        hadron_linkset::linkset_entry!("hadron_block_fs",
            $name: $crate::driver_api::registration::BlockFsEntry = $entry
        );
    };
}

/// Register a virtual filesystem entry in the `.hadron_virtual_fs` linker section.
#[macro_export]
macro_rules! virtual_fs_entry {
    ($name:ident, $entry:expr) => {
        hadron_linkset::linkset_entry!("hadron_virtual_fs",
            $name: $crate::driver_api::registration::VirtualFsEntry = $entry
        );
    };
}

/// Register an initramfs unpacker entry in the `.hadron_initramfs` linker section.
#[macro_export]
macro_rules! initramfs_entry {
    ($name:ident, $entry:expr) => {
        hadron_linkset::linkset_entry!("hadron_initramfs",
            $name: $crate::driver_api::registration::InitramFsEntry = $entry
        );
    };
}
