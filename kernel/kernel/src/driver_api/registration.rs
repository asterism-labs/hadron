//! Linker-section-based driver registration types and macros.
//!
//! Driver crates use the `#[hadron_driver(...)]` attribute macro to declare
//! PCI and platform drivers. Filesystem drivers use [`block_fs_entry!`],
//! [`virtual_fs_entry!`], and [`initramfs_entry!`] to place static entries
//! into dedicated linker sections. The kernel iterates these sections at boot
//! to discover and match drivers to devices, and mount filesystems — no
//! runtime registry needed.

extern crate alloc;

use alloc::sync::Arc;

use hadron_driver_api::acpi_device::AcpiMatchId;
use hadron_driver_api::error::DriverError;
use hadron_driver_api::pci::PciDeviceId;

// Re-export types that moved to hadron-driver-api so existing import paths
// (`hadron_kernel::driver_api::registration::{DeviceSet, ...}`) continue to work.
pub use hadron_driver_api::device_set::{
    DeviceSet, PciDriverRegistration, PlatformDriverRegistration,
};

use super::capability::CapabilityFlags;
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
    pub capabilities: CapabilityFlags,
    /// Called when a matching device is found.
    ///
    /// Receives a [`PciProbeContext`] with typed capability tokens.
    /// Returns a [`PciDriverRegistration`] describing the devices created.
    pub probe: fn(PciProbeContext) -> Result<PciDriverRegistration, DriverError>,
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
    pub id_table: &'static [AcpiMatchId],
    /// Declared capability flags for runtime auditing.
    pub capabilities: CapabilityFlags,
    /// Probe function called when a matching device is found.
    ///
    /// Receives a [`PlatformProbeContext`] with typed capability tokens.
    /// Returns a [`PlatformDriverRegistration`] describing the devices created.
    pub probe: fn(PlatformProbeContext) -> Result<PlatformDriverRegistration, DriverError>,
}

// ---------------------------------------------------------------------------
// Filesystem entries (depend on kernel's fs module)
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
        alloc::boxed::Box<dyn hadron_driver_api::dyn_dispatch::DynBlockDevice>,
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
