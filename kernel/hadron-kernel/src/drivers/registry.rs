//! Driver discovery via linker sections and device-to-driver matching.
//!
//! At link time, driver crates place [`PciDriverEntry`] and [`PlatformDriverEntry`]
//! structs into dedicated linker sections. This module reads those sections and
//! matches entries against discovered devices.

use crate::driver_api::pci::PciDeviceInfo;
use crate::driver_api::probe_context;
use crate::driver_api::registration::{
    BlockFsEntry, InitramFsEntry, PciDriverEntry, PlatformDriverEntry, VirtualFsEntry,
};

// Linker-defined section boundaries (set in the linker script).
unsafe extern "C" {
    static __hadron_pci_drivers_start: u8;
    static __hadron_pci_drivers_end: u8;
    static __hadron_platform_drivers_start: u8;
    static __hadron_platform_drivers_end: u8;
    static __hadron_block_fs_start: u8;
    static __hadron_block_fs_end: u8;
    static __hadron_virtual_fs_start: u8;
    static __hadron_virtual_fs_end: u8;
    static __hadron_initramfs_start: u8;
    static __hadron_initramfs_end: u8;
}

/// Returns all PCI driver entries from the `.hadron_pci_drivers` linker section.
pub fn pci_driver_entries() -> &'static [PciDriverEntry] {
    unsafe {
        let start = core::ptr::addr_of!(__hadron_pci_drivers_start).cast::<PciDriverEntry>();
        let end = core::ptr::addr_of!(__hadron_pci_drivers_end).cast::<PciDriverEntry>();
        let count = end.offset_from(start) as usize;
        if count == 0 {
            return &[];
        }
        core::slice::from_raw_parts(start, count)
    }
}

/// Returns all platform driver entries from the `.hadron_platform_drivers` linker section.
pub fn platform_driver_entries() -> &'static [PlatformDriverEntry] {
    unsafe {
        let start =
            core::ptr::addr_of!(__hadron_platform_drivers_start).cast::<PlatformDriverEntry>();
        let end = core::ptr::addr_of!(__hadron_platform_drivers_end).cast::<PlatformDriverEntry>();
        let count = end.offset_from(start) as usize;
        if count == 0 {
            return &[];
        }
        core::slice::from_raw_parts(start, count)
    }
}

/// Returns all block filesystem entries from the `.hadron_block_fs` linker section.
pub fn block_fs_entries() -> &'static [BlockFsEntry] {
    unsafe {
        let start = core::ptr::addr_of!(__hadron_block_fs_start).cast::<BlockFsEntry>();
        let end = core::ptr::addr_of!(__hadron_block_fs_end).cast::<BlockFsEntry>();
        let count = end.offset_from(start) as usize;
        if count == 0 {
            return &[];
        }
        core::slice::from_raw_parts(start, count)
    }
}

/// Returns all virtual filesystem entries from the `.hadron_virtual_fs` linker section.
pub fn virtual_fs_entries() -> &'static [VirtualFsEntry] {
    unsafe {
        let start = core::ptr::addr_of!(__hadron_virtual_fs_start).cast::<VirtualFsEntry>();
        let end = core::ptr::addr_of!(__hadron_virtual_fs_end).cast::<VirtualFsEntry>();
        let count = end.offset_from(start) as usize;
        if count == 0 {
            return &[];
        }
        core::slice::from_raw_parts(start, count)
    }
}

/// Returns all initramfs entries from the `.hadron_initramfs` linker section.
pub fn initramfs_entries() -> &'static [InitramFsEntry] {
    unsafe {
        let start = core::ptr::addr_of!(__hadron_initramfs_start).cast::<InitramFsEntry>();
        let end = core::ptr::addr_of!(__hadron_initramfs_end).cast::<InitramFsEntry>();
        let count = end.offset_from(start) as usize;
        if count == 0 {
            return &[];
        }
        core::slice::from_raw_parts(start, count)
    }
}

/// Matches discovered PCI devices against registered PCI drivers.
///
/// For each driver entry, iterates its ID table and calls `probe` with a
/// [`PciProbeContext`](crate::driver_api::probe_context::PciProbeContext)
/// on the first matching device. Registers resulting devices in the device registry.
pub fn match_pci_drivers(devices: &[PciDeviceInfo]) {
    let entries = pci_driver_entries();
    for entry in entries {
        for device in devices {
            for id in entry.id_table {
                if id.matches(device) {
                    crate::kprintln!(
                        "PCI: matched {} -> driver '{}'",
                        device.address,
                        entry.name,
                    );
                    let ctx = probe_context::pci_probe_context(device);
                    match (entry.probe)(ctx) {
                        Ok(registration) => {
                            crate::kprintln!("PCI: driver '{}' probe OK", entry.name);
                            crate::drivers::device_registry::with_device_registry_mut(|dr| {
                                dr.register_driver(
                                    entry.name,
                                    registration.devices,
                                    registration.lifecycle,
                                );
                            });
                        }
                        Err(e) => {
                            crate::kprintln!(
                                "PCI: driver '{}' probe failed: {}",
                                entry.name,
                                e,
                            );
                        }
                    }
                    break;
                }
            }
        }
    }
}

/// Matches platform devices against registered platform drivers.
///
/// Compares each platform device's compatible string against driver entries.
/// Calls `init` with a [`PlatformProbeContext`](crate::driver_api::probe_context::PlatformProbeContext)
/// on the first match. Registers resulting devices in the device registry.
pub fn match_platform_drivers(devices: &[(&str, &str)]) {
    let entries = platform_driver_entries();
    for &(name, compatible) in devices {
        for entry in entries {
            if entry.compatible == compatible {
                crate::kprintln!("Platform: matched '{}' -> driver '{}'", name, entry.name,);
                let ctx = probe_context::platform_probe_context();
                match (entry.init)(ctx) {
                    Ok(registration) => {
                        crate::kprintln!("Platform: driver '{}' init OK", entry.name);
                        crate::drivers::device_registry::with_device_registry_mut(|dr| {
                            dr.register_driver(
                                entry.name,
                                registration.devices,
                                registration.lifecycle,
                            );
                        });
                    }
                    Err(e) => {
                        crate::kprintln!("Platform: driver '{}' init failed: {}", name, e,);
                    }
                }
                break;
            }
        }
    }
}
