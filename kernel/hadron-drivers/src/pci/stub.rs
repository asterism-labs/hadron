//! Minimal PCI stub driver for the ICH9 LPC/ISA bridge.
//!
//! Matches the Intel ICH9 LPC controller (vendor 0x8086, device 0x2918) which
//! is always present on QEMU's Q35 chipset. Validates the PCI driver
//! registration and matching pipeline end-to-end.

use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::pci::{PciDeviceId, PciDeviceInfo};

/// PCI device ID table for the ICH9 LPC/ISA bridge.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 1] = [
    PciDeviceId::new(0x8086, 0x2918), // ICH9 LPC/ISA bridge
];

#[cfg(target_os = "none")]
hadron_kernel::pci_driver_entry!(
    PCI_STUB_DRIVER,
    hadron_kernel::driver_api::registration::PciDriverEntry {
        name: "ich9-lpc-stub",
        id_table: &ID_TABLE,
        probe: ich9_lpc_probe,
    }
);

#[cfg(target_os = "none")]
fn ich9_lpc_probe(
    _info: &PciDeviceInfo,
    _services: &'static dyn hadron_kernel::driver_api::services::KernelServices,
) -> Result<(), DriverError> {
    // Stub: validates the entire PCI pipeline. No actual hardware setup needed.
    Ok(())
}
