//! PCI Configuration Access Mechanism (CAM) via legacy I/O ports.
//!
//! Uses ports `0xCF8` (CONFIG_ADDRESS) and `0xCFC` (CONFIG_DATA) to access
//! the 256-byte configuration space of each PCI function. This works on all
//! x86 systems including QEMU q35.

use crate::arch::x86_64::Port;

const CONFIG_ADDRESS: u16 = 0x0CF8;
const CONFIG_DATA: u16 = 0x0CFC;

/// PCI Configuration Access Mechanism (CAM) via I/O ports 0xCF8/0xCFC.
pub struct PciCam;

impl PciCam {
    /// Builds the CONFIG_ADDRESS value for a given BDF + register offset.
    #[inline]
    fn make_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        (1u32 << 31) // enable bit
            | (u32::from(bus) << 16)
            | (u32::from(device & 0x1F) << 11)
            | (u32::from(function & 0x07) << 8)
            | (u32::from(offset) & 0xFC) // dword-aligned
    }

    /// Reads a 32-bit value from PCI config space.
    ///
    /// # Safety
    ///
    /// The caller must ensure the BDF address refers to a valid PCI device
    /// and that no concurrent config space access is in progress.
    pub unsafe fn read_u32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        let addr_port = Port::<u32>::new(CONFIG_ADDRESS);
        let data_port = Port::<u32>::new(CONFIG_DATA);
        unsafe {
            addr_port.write(Self::make_address(bus, device, function, offset));
            data_port.read()
        }
    }

    /// Reads a 16-bit value from PCI config space.
    ///
    /// # Safety
    ///
    /// Same requirements as [`read_u32`](Self::read_u32).
    pub unsafe fn read_u16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
        let dword = unsafe { Self::read_u32(bus, device, function, offset) };
        let shift = ((offset & 2) as u32) * 8;
        (dword >> shift) as u16
    }

    /// Reads an 8-bit value from PCI config space.
    ///
    /// # Safety
    ///
    /// Same requirements as [`read_u32`](Self::read_u32).
    pub unsafe fn read_u8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
        let dword = unsafe { Self::read_u32(bus, device, function, offset) };
        let shift = ((offset & 3) as u32) * 8;
        (dword >> shift) as u8
    }

    /// Writes a 32-bit value to PCI config space.
    ///
    /// # Safety
    ///
    /// PCI config space writes can have side effects on hardware.
    pub unsafe fn write_u32(bus: u8, device: u8, function: u8, offset: u8, val: u32) {
        let addr_port = Port::<u32>::new(CONFIG_ADDRESS);
        let data_port = Port::<u32>::new(CONFIG_DATA);
        unsafe {
            addr_port.write(Self::make_address(bus, device, function, offset));
            data_port.write(val);
        }
    }
}

impl hadron_pci::PciConfigAccess for PciCam {
    unsafe fn read_u32(&self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        unsafe { PciCam::read_u32(bus, device, function, offset) }
    }

    unsafe fn write_u32(&self, bus: u8, device: u8, function: u8, offset: u8, val: u32) {
        unsafe { PciCam::write_u32(bus, device, function, offset, val) }
    }
}
