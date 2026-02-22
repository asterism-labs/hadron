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

hadron_linkset::declare_linkset! {
    /// Returns all PCI driver entries from the `.hadron_pci_drivers` linker section.
    pub fn pci_driver_entries() -> [PciDriverEntry],
    section = "hadron_pci_drivers"
}

hadron_linkset::declare_linkset! {
    /// Returns all platform driver entries from the `.hadron_platform_drivers` linker section.
    pub fn platform_driver_entries() -> [PlatformDriverEntry],
    section = "hadron_platform_drivers"
}

hadron_linkset::declare_linkset! {
    /// Returns all block filesystem entries from the `.hadron_block_fs` linker section.
    pub fn block_fs_entries() -> [BlockFsEntry],
    section = "hadron_block_fs"
}

hadron_linkset::declare_linkset! {
    /// Returns all virtual filesystem entries from the `.hadron_virtual_fs` linker section.
    pub fn virtual_fs_entries() -> [VirtualFsEntry],
    section = "hadron_virtual_fs"
}

hadron_linkset::declare_linkset! {
    /// Returns all initramfs entries from the `.hadron_initramfs` linker section.
    pub fn initramfs_entries() -> [InitramFsEntry],
    section = "hadron_initramfs"
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
                    crate::kprintln!("PCI: matched {} -> driver '{}'", device.address, entry.name,);
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
                            crate::kprintln!("PCI: driver '{}' probe failed: {}", entry.name, e,);
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
