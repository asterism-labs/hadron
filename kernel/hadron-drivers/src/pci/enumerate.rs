//! PCI bus enumeration.
//!
//! Thin wrapper around `hadron_pci::enumerate` that uses the local static CAM.

use alloc::vec::Vec;
use hadron_kernel::driver_api::pci::PciDeviceInfo;

use super::cam::CAM;

/// Enumerates all PCI devices across all host-controller buses.
pub fn enumerate() -> Vec<PciDeviceInfo> {
    hadron_pci::enumerate::enumerate(&CAM)
}

/// Re-export class name lookup.
pub use hadron_pci::class_name;
