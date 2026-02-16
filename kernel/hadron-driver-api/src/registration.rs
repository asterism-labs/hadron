//! Linker-section-based driver registration types and macros.
//!
//! Driver crates use [`pci_driver_entry!`] and [`platform_driver_entry!`] to place
//! static entries into dedicated linker sections. The kernel iterates these sections
//! at boot to discover and match drivers to devices — no runtime registry needed.

use crate::error::DriverError;
use crate::pci::{PciDeviceId, PciDeviceInfo};
use crate::services::KernelServices;

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

// SAFETY: These are repr(C) structs containing only references to 'static data
// and function pointers, all of which are inherently safe to share across threads.
unsafe impl Sync for PciDriverEntry {}
unsafe impl Sync for PlatformDriverEntry {}

/// Register a PCI driver entry in the `.hadron_pci_drivers` linker section.
///
/// # Example
///
/// ```ignore
/// hadron_driver_api::pci_driver_entry!(MY_DRIVER, PciDriverEntry {
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
        static $name: $crate::registration::PciDriverEntry = $entry;
    };
}

/// Register a platform driver entry in the `.hadron_platform_drivers` linker section.
///
/// # Example
///
/// ```ignore
/// hadron_driver_api::platform_driver_entry!(MY_DRIVER, PlatformDriverEntry {
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
        static $name: $crate::registration::PlatformDriverEntry = $entry;
    };
}
