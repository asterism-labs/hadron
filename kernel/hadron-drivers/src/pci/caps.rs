//! PCI capability linked-list walker.
//!
//! Walks the PCI capability linked list starting from the Capabilities Pointer
//! register (offset 0x34), parsing each capability header. Supports VirtIO PCI
//! capabilities (cap ID 0x09) and MSI-X capabilities (cap ID 0x11).

use super::cam::{regs, PciCam};
use hadron_driver_api::pci::PciAddress;

/// A raw PCI capability header: capability ID and its config-space offset.
#[derive(Debug, Clone, Copy)]
pub struct RawCapability {
    /// PCI capability ID (e.g., 0x09 for vendor-specific, 0x11 for MSI-X).
    pub id: u8,
    /// Config-space offset of this capability header.
    pub offset: u8,
}

/// Iterator over PCI capabilities in a device's config space.
pub struct CapabilityIter {
    bus: u8,
    device: u8,
    function: u8,
    next_offset: u8,
}

impl Iterator for CapabilityIter {
    type Item = RawCapability;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next_offset != 0 {
            let offset = self.next_offset & 0xFC; // dword-aligned
            if offset == 0 {
                break;
            }

            // SAFETY: We are reading PCI config space of a device that was
            // previously enumerated and confirmed to exist.
            let cap_id =
                unsafe { PciCam::read_u8(self.bus, self.device, self.function, offset) };
            let next =
                unsafe { PciCam::read_u8(self.bus, self.device, self.function, offset + 1) };

            self.next_offset = next;

            return Some(RawCapability { id: cap_id, offset });
        }
        None
    }
}

/// Returns an iterator over all PCI capabilities for the given device.
///
/// Returns `None` if the device does not have a capabilities list (status
/// register bit 4 is clear).
pub fn walk_capabilities(addr: &PciAddress) -> Option<CapabilityIter> {
    // SAFETY: Reading status register of an enumerated PCI device.
    let status = unsafe { PciCam::read_u16(addr.bus, addr.device, addr.function, regs::STATUS) };

    if status & regs::STATUS_CAPABILITIES_LIST == 0 {
        return None;
    }

    // SAFETY: Reading capabilities pointer of an enumerated PCI device.
    let cap_ptr = unsafe {
        PciCam::read_u8(addr.bus, addr.device, addr.function, regs::CAPABILITIES_PTR)
    };

    Some(CapabilityIter {
        bus: addr.bus,
        device: addr.device,
        function: addr.function,
        next_offset: cap_ptr,
    })
}

// ---------------------------------------------------------------------------
// VirtIO PCI capability
// ---------------------------------------------------------------------------

/// VirtIO PCI capability config type values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VirtioPciCfgType {
    /// Common configuration.
    CommonCfg = 1,
    /// Notifications.
    NotifyCfg = 2,
    /// ISR status.
    IsrCfg = 3,
    /// Device-specific configuration.
    DeviceCfg = 4,
    /// PCI configuration access.
    PciCfg = 5,
}

impl VirtioPciCfgType {
    /// Converts a raw byte to a config type, if valid.
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::CommonCfg),
            2 => Some(Self::NotifyCfg),
            3 => Some(Self::IsrCfg),
            4 => Some(Self::DeviceCfg),
            5 => Some(Self::PciCfg),
            _ => None,
        }
    }
}

/// Parsed VirtIO PCI capability structure.
#[derive(Debug, Clone, Copy)]
pub struct VirtioPciCap {
    /// Configuration structure type.
    pub cfg_type: VirtioPciCfgType,
    /// BAR index (0-5) containing this config structure.
    pub bar: u8,
    /// Offset within the BAR.
    pub offset: u32,
    /// Length of the config structure.
    pub length: u32,
    /// Raw config-space offset of this capability (for notify_off_multiplier).
    pub cap_offset: u8,
}

/// Reads a VirtIO PCI capability at the given config-space offset.
///
/// Returns `None` if the capability type is not recognized.
pub fn read_virtio_pci_cap(addr: &PciAddress, cap_offset: u8) -> Option<VirtioPciCap> {
    let (bus, dev, func) = (addr.bus, addr.device, addr.function);

    // SAFETY: Reading PCI config space of an enumerated device at known
    // capability offsets.
    let cfg_type_raw = unsafe { PciCam::read_u8(bus, dev, func, cap_offset + 3) };
    let cfg_type = VirtioPciCfgType::from_u8(cfg_type_raw)?;

    let bar = unsafe { PciCam::read_u8(bus, dev, func, cap_offset + 4) };
    let offset = unsafe { PciCam::read_u32(bus, dev, func, cap_offset + 8) };
    let length = unsafe { PciCam::read_u32(bus, dev, func, cap_offset + 12) };

    Some(VirtioPciCap {
        cfg_type,
        bar,
        offset,
        length,
        cap_offset,
    })
}

// ---------------------------------------------------------------------------
// MSI-X capability
// ---------------------------------------------------------------------------

/// Parsed MSI-X capability from PCI config space.
#[derive(Debug, Clone, Copy)]
pub struct MsixCapability {
    /// Config-space offset of the MSI-X capability header.
    pub cap_offset: u8,
    /// Number of MSI-X table entries (table size = this + 1).
    pub table_size: u16,
    /// BAR index containing the MSI-X table.
    pub table_bar: u8,
    /// Byte offset of the MSI-X table within the BAR.
    pub table_offset: u32,
    /// BAR index containing the PBA (Pending Bit Array).
    pub pba_bar: u8,
    /// Byte offset of the PBA within the BAR.
    pub pba_offset: u32,
}

/// Reads an MSI-X capability at the given config-space offset.
pub fn read_msix_cap(addr: &PciAddress, cap_offset: u8) -> MsixCapability {
    let (bus, dev, func) = (addr.bus, addr.device, addr.function);

    // SAFETY: Reading PCI config space of an enumerated device at known
    // MSI-X capability offsets.
    let msg_control = unsafe { PciCam::read_u16(bus, dev, func, cap_offset + 2) };
    let table_bir_offset = unsafe { PciCam::read_u32(bus, dev, func, cap_offset + 4) };
    let pba_bir_offset = unsafe { PciCam::read_u32(bus, dev, func, cap_offset + 8) };

    // Table size is bits 10:0 of message control.
    let table_size = msg_control & 0x7FF;
    // BIR = bits 2:0, offset = bits 31:3 (shifted left by 3).
    let table_bar = (table_bir_offset & 0x7) as u8;
    let table_offset = table_bir_offset & !0x7;
    let pba_bar = (pba_bir_offset & 0x7) as u8;
    let pba_offset = pba_bir_offset & !0x7;

    MsixCapability {
        cap_offset,
        table_size,
        table_bar,
        table_offset,
        pba_bar,
        pba_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtio_pci_cfg_type_from_u8() {
        assert_eq!(
            VirtioPciCfgType::from_u8(1),
            Some(VirtioPciCfgType::CommonCfg)
        );
        assert_eq!(
            VirtioPciCfgType::from_u8(2),
            Some(VirtioPciCfgType::NotifyCfg)
        );
        assert_eq!(
            VirtioPciCfgType::from_u8(3),
            Some(VirtioPciCfgType::IsrCfg)
        );
        assert_eq!(
            VirtioPciCfgType::from_u8(4),
            Some(VirtioPciCfgType::DeviceCfg)
        );
        assert_eq!(
            VirtioPciCfgType::from_u8(5),
            Some(VirtioPciCfgType::PciCfg)
        );
        assert_eq!(VirtioPciCfgType::from_u8(0), None);
        assert_eq!(VirtioPciCfgType::from_u8(6), None);
    }
}
