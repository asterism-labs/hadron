//! PCI bus types for device enumeration and driver matching.

/// PCI bus/device/function address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciAddress {
    /// Bus number (0-255).
    pub bus: u8,
    /// Device number (0-31).
    pub device: u8,
    /// Function number (0-7).
    pub function: u8,
}

impl core::fmt::Display for PciAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:02x}:{:02x}.{}", self.bus, self.device, self.function)
    }
}

/// Wildcard value for PCI ID matching: matches any vendor/device ID.
pub const PCI_ANY_ID: u16 = 0xFFFF;

/// PCI device ID for driver-to-device matching.
#[derive(Debug, Clone, Copy)]
pub struct PciDeviceId {
    /// Vendor ID (`PCI_ANY_ID` = wildcard).
    pub vendor: u16,
    /// Device ID (`PCI_ANY_ID` = wildcard).
    pub device: u16,
    /// Subsystem vendor ID (`PCI_ANY_ID` = wildcard).
    pub subvendor: u16,
    /// Subsystem device ID (`PCI_ANY_ID` = wildcard).
    pub subdevice: u16,
    /// Class code: `(class << 16) | (subclass << 8) | prog_if`.
    pub class: u32,
    /// Mask applied to class before comparison (0 = ignore class).
    pub class_mask: u32,
}

impl PciDeviceId {
    /// Creates an ID entry matching a specific vendor/device pair.
    #[must_use]
    pub const fn new(vendor: u16, device: u16) -> Self {
        Self {
            vendor,
            device,
            subvendor: PCI_ANY_ID,
            subdevice: PCI_ANY_ID,
            class: 0,
            class_mask: 0,
        }
    }

    /// Creates an ID entry matching a PCI class/subclass.
    #[must_use]
    pub const fn with_class(class: u8, subclass: u8) -> Self {
        Self {
            vendor: PCI_ANY_ID,
            device: PCI_ANY_ID,
            subvendor: PCI_ANY_ID,
            subdevice: PCI_ANY_ID,
            class: ((class as u32) << 16) | ((subclass as u32) << 8),
            class_mask: 0xFFFF00,
        }
    }

    /// Creates an ID entry matching a PCI class, subclass, and programming interface.
    #[must_use]
    pub const fn with_class_progif(class: u8, subclass: u8, prog_if: u8) -> Self {
        Self {
            vendor: PCI_ANY_ID,
            device: PCI_ANY_ID,
            subvendor: PCI_ANY_ID,
            subdevice: PCI_ANY_ID,
            class: ((class as u32) << 16) | ((subclass as u32) << 8) | (prog_if as u32),
            class_mask: 0xFFFFFF,
        }
    }

    /// Returns `true` if this ID entry matches the given device info.
    #[must_use]
    pub fn matches(&self, info: &PciDeviceInfo) -> bool {
        if self.vendor != PCI_ANY_ID && self.vendor != info.vendor_id {
            return false;
        }
        if self.device != PCI_ANY_ID && self.device != info.device_id {
            return false;
        }
        if self.subvendor != PCI_ANY_ID && self.subvendor != info.subsystem_vendor_id {
            return false;
        }
        if self.subdevice != PCI_ANY_ID && self.subdevice != info.subsystem_device_id {
            return false;
        }
        if self.class_mask != 0 {
            let dev_class =
                ((info.class as u32) << 16) | ((info.subclass as u32) << 8) | (info.prog_if as u32);
            if (dev_class & self.class_mask) != (self.class & self.class_mask) {
                return false;
            }
        }
        true
    }
}

/// Decoded PCI Base Address Register.
#[derive(Debug, Clone, Copy)]
pub enum PciBar {
    /// Memory-mapped BAR.
    Memory {
        /// Base physical address.
        base: u64,
        /// Size in bytes.
        size: u64,
        /// Whether the region is prefetchable.
        prefetchable: bool,
        /// Whether this is a 64-bit BAR (consumes two BAR slots).
        is_64bit: bool,
    },
    /// I/O port BAR.
    Io {
        /// Base I/O port address.
        base: u32,
        /// Size in bytes.
        size: u32,
    },
    /// BAR slot is unused or consumed by the upper half of a 64-bit BAR.
    Unused,
}

/// Full information about a discovered PCI device.
#[derive(Debug, Clone, Copy)]
pub struct PciDeviceInfo {
    /// Bus/device/function address.
    pub address: PciAddress,
    /// Vendor ID.
    pub vendor_id: u16,
    /// Device ID.
    pub device_id: u16,
    /// Revision ID.
    pub revision: u8,
    /// Programming interface byte.
    pub prog_if: u8,
    /// Subclass code.
    pub subclass: u8,
    /// Class code.
    pub class: u8,
    /// Header type (bits 0-6), multi-function flag (bit 7).
    pub header_type: u8,
    /// Subsystem vendor ID.
    pub subsystem_vendor_id: u16,
    /// Subsystem device ID.
    pub subsystem_device_id: u16,
    /// Interrupt line (IRQ number configured by firmware).
    pub interrupt_line: u8,
    /// Interrupt pin (0 = none, 1 = INTA, ..., 4 = INTD).
    pub interrupt_pin: u8,
    /// Base Address Registers (up to 6 for type 0, 2 for type 1).
    pub bars: [PciBar; 6],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device_info(vendor: u16, device: u16, class: u8, subclass: u8) -> PciDeviceInfo {
        PciDeviceInfo {
            address: PciAddress {
                bus: 0,
                device: 0,
                function: 0,
            },
            vendor_id: vendor,
            device_id: device,
            revision: 0,
            prog_if: 0,
            subclass,
            class,
            header_type: 0,
            subsystem_vendor_id: 0,
            subsystem_device_id: 0,
            interrupt_line: 0,
            interrupt_pin: 0,
            bars: [PciBar::Unused; 6],
        }
    }

    #[test]
    fn exact_vendor_device_match() {
        let id = PciDeviceId::new(0x8086, 0x2918);
        let info = make_device_info(0x8086, 0x2918, 0, 0);
        assert!(id.matches(&info));
    }

    #[test]
    fn vendor_mismatch() {
        let id = PciDeviceId::new(0x8086, 0x2918);
        let info = make_device_info(0x1234, 0x2918, 0, 0);
        assert!(!id.matches(&info));
    }

    #[test]
    fn device_mismatch() {
        let id = PciDeviceId::new(0x8086, 0x2918);
        let info = make_device_info(0x8086, 0x1111, 0, 0);
        assert!(!id.matches(&info));
    }

    #[test]
    fn wildcard_vendor() {
        let id = PciDeviceId {
            vendor: PCI_ANY_ID,
            device: 0x2918,
            subvendor: PCI_ANY_ID,
            subdevice: PCI_ANY_ID,
            class: 0,
            class_mask: 0,
        };
        let info = make_device_info(0x1234, 0x2918, 0, 0);
        assert!(id.matches(&info));
    }

    #[test]
    fn class_match() {
        let id = PciDeviceId::with_class(0x06, 0x01); // ISA bridge
        let info = make_device_info(0x8086, 0x2918, 0x06, 0x01);
        assert!(id.matches(&info));
    }

    #[test]
    fn class_mismatch() {
        let id = PciDeviceId::with_class(0x06, 0x01);
        let info = make_device_info(0x8086, 0x2918, 0x02, 0x00);
        assert!(!id.matches(&info));
    }
}
