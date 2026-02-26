//! PCI capability linked-list walker.
//!
//! Thin wrappers around `hadron_pci::caps` that use the local static CAM.

use hadron_kernel::driver_api::pci::PciAddress;

use super::cam::CAM;

// Re-export all types from hadron_pci::caps.
pub use hadron_pci::caps::{MsixCapability, RawCapability, VirtioPciCap, VirtioPciCfgType};

/// Returns an iterator over all PCI capabilities for the given device.
pub fn walk_capabilities(
    addr: &PciAddress,
) -> Option<hadron_pci::caps::CapabilityIter<'static, super::cam::PciCam>> {
    hadron_pci::caps::walk_capabilities(&CAM, addr)
}

/// Reads a VirtIO PCI capability at the given config-space offset.
pub fn read_virtio_pci_cap(addr: &PciAddress, cap_offset: u8) -> Option<VirtioPciCap> {
    hadron_pci::caps::read_virtio_pci_cap(&CAM, addr, cap_offset)
}

/// Reads an MSI-X capability at the given config-space offset.
pub fn read_msix_cap(addr: &PciAddress, cap_offset: u8) -> MsixCapability {
    hadron_pci::caps::read_msix_cap(&CAM, addr, cap_offset)
}
