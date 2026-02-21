//! Minimal PCI stub driver for the ICH9 LPC/ISA bridge.
//!
//! Matches the Intel ICH9 LPC controller (vendor 0x8086, device 0x2918) which
//! is always present on QEMU's Q35 chipset. Validates the PCI driver
//! registration and matching pipeline end-to-end.

use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::pci::PciDeviceId;

/// PCI device ID table for the ICH9 LPC/ISA bridge.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 1] = [
    PciDeviceId::new(0x8086, 0x2918), // ICH9 LPC/ISA bridge
];

/// ICH9 LPC/ISA bridge stub driver registration type.
struct Ich9LpcStubDriver;

#[hadron_driver_macros::hadron_driver(
    name = "ich9-lpc-stub",
    kind = pci,
    capabilities = [],
    pci_ids = &ID_TABLE,
)]
impl Ich9LpcStubDriver {
    fn probe(
        _ctx: DriverContext,
    ) -> Result<hadron_kernel::driver_api::registration::PciDriverRegistration, DriverError> {
        use hadron_kernel::driver_api::registration::{DeviceSet, PciDriverRegistration};

        // Stub: validates the entire PCI pipeline. No actual hardware setup needed.
        Ok(PciDriverRegistration {
            devices: DeviceSet::new(),
            lifecycle: None,
        })
    }
}
