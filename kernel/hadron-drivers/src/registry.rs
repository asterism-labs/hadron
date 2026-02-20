//! Driver discovery via linker sections and device-to-driver matching.
//!
//! At link time, driver crates place [`PciDriverEntry`] and [`PlatformDriverEntry`]
//! structs into dedicated linker sections. This module reads those sections and
//! matches entries against discovered devices.

use hadron_kernel::driver_api::pci::PciDeviceInfo;
use hadron_kernel::driver_api::registration::{PciDriverEntry, PlatformDriverEntry};
use hadron_kernel::driver_api::services::KernelServices;

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
/// For each driver entry, iterates its ID table and calls `probe` on the
/// first matching device. Logs all matches and probe results.
pub fn match_pci_drivers(devices: &[PciDeviceInfo], services: &'static dyn KernelServices) {
    let entries = pci_driver_entries();
    for entry in entries {
        for device in devices {
            for id in entry.id_table {
                if id.matches(device) {
                    hadron_kernel::kprintln!(
                        "PCI: matched {} -> driver '{}'",
                        device.address,
                        entry.name,
                    );
                    match (entry.probe)(device, services) {
                        Ok(()) => {
                            hadron_kernel::kprintln!("PCI: driver '{}' probe OK", entry.name);
                        }
                        Err(e) => {
                            hadron_kernel::kprintln!(
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
/// Calls `init` on the first match.
pub fn match_platform_drivers(devices: &[(&str, &str)], services: &'static dyn KernelServices) {
    let entries = platform_driver_entries();
    for &(name, compatible) in devices {
        for entry in entries {
            if entry.compatible == compatible {
                hadron_kernel::kprintln!("Platform: matched '{}' -> driver '{}'", name, entry.name,);
                match (entry.init)(services) {
                    Ok(()) => {
                        hadron_kernel::kprintln!("Platform: driver '{}' init OK", entry.name);
                    }
                    Err(e) => {
                        hadron_kernel::kprintln!("Platform: driver '{}' init failed: {}", name, e,);
                    }
                }
                break;
            }
        }
    }
}
