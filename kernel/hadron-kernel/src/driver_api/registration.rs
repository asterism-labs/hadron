//! Linker-section-based driver registration types and macros.
//!
//! Driver crates use [`pci_driver_entry!`], [`platform_driver_entry!`],
//! [`block_fs_entry!`], [`virtual_fs_entry!`], and [`initramfs_entry!`] to place
//! static entries into dedicated linker sections. The kernel iterates these sections
//! at boot to discover and match drivers to devices, and mount filesystems — no
//! runtime registry needed.

extern crate alloc;

use alloc::sync::Arc;

use super::error::DriverError;
use super::pci::{PciDeviceId, PciDeviceInfo};
use super::services::KernelServices;

/// PCI driver entry placed in the `.hadron_pci_drivers` linker section.
///
/// Contains the driver's name, supported device IDs, and a probe function.
/// The kernel iterates these entries to match discovered PCI devices to drivers.
#[repr(C)]
pub struct PciDriverEntry {
    /// Driver name (for logging).
    pub name: &'static str,
    /// Device IDs this driver supports.
    pub id_table: &'static [PciDeviceId],
    /// Called when a matching device is found.
    pub probe: fn(&PciDeviceInfo, &'static dyn KernelServices) -> Result<(), DriverError>,
}

/// Platform driver entry placed in the `.hadron_platform_drivers` linker section.
///
/// Platform drivers are matched by compatible string (e.g., "ns16550").
#[repr(C)]
pub struct PlatformDriverEntry {
    /// Driver name (for logging).
    pub name: &'static str,
    /// Compatible string for matching (e.g., "ns16550").
    pub compatible: &'static str,
    /// Initialization function called when matched.
    pub init: fn(&'static dyn KernelServices) -> Result<(), DriverError>,
}

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

/// Register a PCI driver entry in the `.hadron_pci_drivers` linker section.
///
/// # Example
///
/// ```ignore
/// hadron_kernel::pci_driver_entry!(MY_DRIVER, PciDriverEntry {
///     name: "my_pci_driver",
///     id_table: &[PciDeviceId::new(0x1234, 0x5678)],
///     probe: my_probe_fn,
/// });
/// ```
#[macro_export]
macro_rules! pci_driver_entry {
    ($name:ident, $entry:expr) => {
        #[used]
        #[unsafe(link_section = ".hadron_pci_drivers")]
        static $name: $crate::driver_api::registration::PciDriverEntry = $entry;
    };
}

/// Register a platform driver entry in the `.hadron_platform_drivers` linker section.
///
/// # Example
///
/// ```ignore
/// hadron_kernel::platform_driver_entry!(MY_DRIVER, PlatformDriverEntry {
///     name: "my_platform_driver",
///     compatible: "ns16550",
///     init: my_init_fn,
/// });
/// ```
#[macro_export]
macro_rules! platform_driver_entry {
    ($name:ident, $entry:expr) => {
        #[used]
        #[unsafe(link_section = ".hadron_platform_drivers")]
        static $name: $crate::driver_api::registration::PlatformDriverEntry = $entry;
    };
}

/// Register a block filesystem entry in the `.hadron_block_fs` linker section.
///
/// # Example
///
/// ```ignore
/// hadron_kernel::block_fs_entry!(FAT_FS, BlockFsEntry {
///     name: "fat",
///     mount: fat_mount,
/// });
/// ```
#[macro_export]
macro_rules! block_fs_entry {
    ($name:ident, $entry:expr) => {
        #[used]
        #[unsafe(link_section = ".hadron_block_fs")]
        static $name: $crate::driver_api::registration::BlockFsEntry = $entry;
    };
}

/// Register a virtual filesystem entry in the `.hadron_virtual_fs` linker section.
///
/// # Example
///
/// ```ignore
/// hadron_kernel::virtual_fs_entry!(RAMFS, VirtualFsEntry {
///     name: "ramfs",
///     create: create_ramfs,
/// });
/// ```
#[macro_export]
macro_rules! virtual_fs_entry {
    ($name:ident, $entry:expr) => {
        #[used]
        #[unsafe(link_section = ".hadron_virtual_fs")]
        static $name: $crate::driver_api::registration::VirtualFsEntry = $entry;
    };
}

/// Register an initramfs unpacker entry in the `.hadron_initramfs` linker section.
///
/// # Example
///
/// ```ignore
/// hadron_kernel::initramfs_entry!(CPIO_UNPACKER, InitramFsEntry {
///     name: "cpio",
///     unpack: unpack_cpio,
/// });
/// ```
#[macro_export]
macro_rules! initramfs_entry {
    ($name:ident, $entry:expr) => {
        #[used]
        #[unsafe(link_section = ".hadron_initramfs")]
        static $name: $crate::driver_api::registration::InitramFsEntry = $entry;
    };
}
