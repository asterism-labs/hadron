//! Driver discovery via linker sections and device-to-driver matching.
//!
//! At link time, driver crates place [`PciDriverEntry`] and [`PlatformDriverEntry`]
//! structs into dedicated linker sections. This module reads those sections and
//! matches entries against discovered devices.

use hadron_kernel::driver_api::pci::PciDeviceInfo;
use hadron_kernel::driver_api::probe_context;
use hadron_kernel::driver_api::registration::{PciDriverEntry, PlatformDriverEntry};

// Linker-defined section boundaries (set in the linker script).
unsafe extern "C" {
    static __hadron_pci_drivers_start: u8;
    static __hadron_pci_drivers_end: u8;
    static __hadron_platform_drivers_start: u8;
    static __hadron_platform_drivers_end: u8;
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

/// Matches discovered PCI devices against registered PCI drivers.
///
/// For each driver entry, iterates its ID table and calls `probe` with a
/// [`PciProbeContext`](hadron_kernel::driver_api::probe_context::PciProbeContext)
/// on the first matching device. Registers resulting devices in the device registry.
pub fn match_pci_drivers(devices: &[PciDeviceInfo]) {
    let entries = pci_driver_entries();
    for entry in entries {
        for device in devices {
            for id in entry.id_table {
                if id.matches(device) {
                    hadron_kernel::kprintln!(
                        "PCI: matched {} -> driver '{}' [{}]",
                        device.address,
                        entry.name,
                        entry.capabilities,
                    );
                    let ctx = probe_context::pci_probe_context(device);
                    match (entry.probe)(ctx) {
                        Ok(registration) => {
                            hadron_kernel::kprintln!("PCI: driver '{}' probe OK", entry.name);
                            hadron_kernel::ktrace_subsys!(
                                drivers,
                                "PCI driver '{}' probed for {}",
                                entry.name,
                                device.address
                            );
                            hadron_kernel::drivers::device_registry::with_device_registry_mut(
                                |dr| {
                                    dr.register_driver(
                                        entry.name,
                                        registration.devices,
                                        registration.lifecycle,
                                    );
                                },
                            );
                        }
                        Err(e) => {
                            hadron_kernel::kprintln!(
                                "PCI: driver '{}' probe failed: {}",
                                entry.name,
                                e,
                            );
                            hadron_kernel::ktrace_subsys!(
                                drivers,
                                "PCI driver '{}' probe failed: {}",
                                entry.name,
                                e
                            );
                        }
                    }
                    break;
                }
            }
        }
    }
}

/// Matches ACPI-discovered platform devices against registered platform drivers.
///
/// For each device, compares its `_HID` and `_CID` against each driver's
/// [`AcpiMatchId`] table. Calls `probe` with a [`PlatformProbeContext`] on
/// the first match.
pub fn match_platform_drivers(devices: &[hadron_kernel::driver_api::acpi_device::AcpiDeviceInfo]) {
    let entries = platform_driver_entries();
    for device in devices {
        for entry in entries {
            let matched = entry.id_table.iter().any(|id| {
                id.matches_hid(&device.hid)
                    || device
                        .cid
                        .as_ref()
                        .is_some_and(|cid| id.matches_hid(cid))
            });
            if matched {
                hadron_kernel::kprintln!(
                    "Platform: matched '{}' -> driver '{}' [{}]",
                    device.path,
                    entry.name,
                    entry.capabilities,
                );
                let ctx = probe_context::platform_probe_context(device.clone());
                match (entry.probe)(ctx) {
                    Ok(registration) => {
                        hadron_kernel::kprintln!("Platform: driver '{}' probe OK", entry.name);
                        hadron_kernel::ktrace_subsys!(
                            drivers,
                            "platform driver '{}' probed for {}",
                            entry.name,
                            device.path
                        );
                        hadron_kernel::drivers::device_registry::with_device_registry_mut(|dr| {
                            dr.register_driver(
                                entry.name,
                                registration.devices,
                                registration.lifecycle,
                            );
                        });
                    }
                    Err(e) => {
                        hadron_kernel::kprintln!(
                            "Platform: driver '{}' probe failed: {}",
                            device.path,
                            e,
                        );
                    }
                }
                break;
            }
        }
    }
}
