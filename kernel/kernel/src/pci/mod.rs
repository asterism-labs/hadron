//! PCI bus core: configuration access, enumeration, and capability parsing.
//!
//! Portable PCI logic (enumeration algorithm, capability walking, register
//! constants) lives in the `hadron-pci` crate. This module provides the
//! kernel-specific implementations: legacy CAM I/O ports, ECAM MMIO, and
//! ACPI interrupt routing.

#[cfg(target_arch = "x86_64")]
pub mod cam;
#[cfg(target_arch = "x86_64")]
pub mod ecam;

// Re-export the portable crate for downstream use.
pub use hadron_pci::caps;
pub use hadron_pci::enumerate as enumerate_mod;
pub use hadron_pci::regs;
pub use hadron_pci::{PciConfigAccess, class_name};

/// Enumerates all PCI devices using legacy CAM I/O ports.
#[cfg(target_arch = "x86_64")]
pub fn enumerate() -> alloc::vec::Vec<hadron_driver_api::pci::PciDeviceInfo> {
    let devices = hadron_pci::enumerate::enumerate(&cam::PciCam);
    crate::ktrace_subsys!(pci, "PCI: enumerated {} devices", devices.len());
    devices
}

/// Applies ACPI `_PRT` interrupt routing to enumerated PCI devices.
#[cfg(target_arch = "x86_64")]
pub fn apply_prt_routing(devices: &mut [hadron_driver_api::pci::PciDeviceInfo]) {
    use hadron_acpi::aml::value::AmlValue;

    let prt_entries = crate::arch::x86_64::acpi::Acpi::with_namespace(|ns| {
        ns.devices()
            .find(|d| {
                let raw = match &d.hid {
                    Some(AmlValue::EisaId(id)) => Some(id.raw),
                    Some(AmlValue::Integer(v)) => Some(*v as u32),
                    _ => None,
                };
                raw.is_some_and(|r| {
                    use hadron_acpi::aml::value::EisaId;
                    let decoded = EisaId { raw: r }.decode();
                    &decoded == b"PNP0A03" || &decoded == b"PNP0A08"
                })
            })
            .map(|d| d.prt.clone())
    })
    .flatten()
    .unwrap_or_default();

    if prt_entries.is_empty() {
        return;
    }

    crate::kdebug!("PCI: applying {} _PRT routing entries", prt_entries.len());

    for device in devices.iter_mut() {
        if device.interrupt_pin > 0 {
            let dev_addr = ((device.address.device as u64) << 16) | 0xFFFF;
            let pin = device.interrupt_pin - 1;
            if let Some(entry) = prt_entries
                .iter()
                .find(|e| e.address == dev_addr && e.pin == pin)
            {
                device.gsi = Some(entry.gsi);
            }
        }
    }
}
