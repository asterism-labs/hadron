//! PCI Configuration Access Mechanism (CAM) via legacy I/O ports.
//!
//! Uses ports `0xCF8` (CONFIG_ADDRESS) and `0xCFC` (CONFIG_DATA) to access
//! the 256-byte configuration space of each PCI function. This works on all
//! x86 systems including QEMU q35.

use hadron_kernel::arch::x86_64::Port;

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
        // Extract the correct 16-bit half based on bit 1 of offset.
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
        // Extract the correct byte based on bits 0-1 of offset.
        let shift = ((offset & 3) as u32) * 8;
        (dword >> shift) as u8
    }

    /// Writes a 32-bit value to PCI config space.
    ///
    /// # Safety
    ///
    /// PCI config space writes can have side effects on hardware. The caller
    /// must ensure the write is safe for the target register.
    pub unsafe fn write_u32(bus: u8, device: u8, function: u8, offset: u8, val: u32) {
        let addr_port = Port::<u32>::new(CONFIG_ADDRESS);
        let data_port = Port::<u32>::new(CONFIG_DATA);
        unsafe {
            addr_port.write(Self::make_address(bus, device, function, offset));
            data_port.write(val);
        }
    }
}

/// Standard PCI configuration space register offsets.
pub mod regs {
    /// Vendor ID (16-bit, offset 0x00).
    pub const VENDOR_ID: u8 = 0x00;
    /// Device ID (16-bit, offset 0x02).
    pub const DEVICE_ID: u8 = 0x02;
    /// Command register (16-bit, offset 0x04).
    pub const COMMAND: u8 = 0x04;
    /// Status register (16-bit, offset 0x06).
    pub const STATUS: u8 = 0x06;
    /// Revision ID (8-bit, offset 0x08).
    pub const REVISION: u8 = 0x08;
    /// Programming Interface (8-bit, offset 0x09).
    pub const PROG_IF: u8 = 0x09;
    /// Subclass code (8-bit, offset 0x0A).
    pub const SUBCLASS: u8 = 0x0A;
    /// Class code (8-bit, offset 0x0B).
    pub const CLASS: u8 = 0x0B;
    /// Header type (8-bit, offset 0x0E). Bit 7 = multi-function.
    pub const HEADER_TYPE: u8 = 0x0E;
    /// Base Address Register 0 (32-bit, offset 0x10). BAR1-5 at +4 intervals.
    pub const BAR0: u8 = 0x10;
    /// Subsystem Vendor ID (16-bit, offset 0x2C).
    pub const SUBSYSTEM_VENDOR_ID: u8 = 0x2C;
    /// Subsystem Device ID (16-bit, offset 0x2E).
    pub const SUBSYSTEM_DEVICE_ID: u8 = 0x2E;
    /// Interrupt Line (8-bit, offset 0x3C).
    pub const INTERRUPT_LINE: u8 = 0x3C;
    /// Interrupt Pin (8-bit, offset 0x3D).
    pub const INTERRUPT_PIN: u8 = 0x3D;
    /// Secondary Bus Number (8-bit, offset 0x19) — PCI-to-PCI bridge only.
    pub const SECONDARY_BUS: u8 = 0x19;
    /// Capabilities Pointer (8-bit, offset 0x34) — pointer to first capability.
    pub const CAPABILITIES_PTR: u8 = 0x34;

    // -- Status register bits -------------------------------------------------

    /// Bit 4 of the Status register: capabilities list present.
    pub const STATUS_CAPABILITIES_LIST: u16 = 1 << 4;

    // -- PCI capability IDs ---------------------------------------------------

    /// MSI-X capability ID.
    pub const CAP_ID_MSIX: u8 = 0x11;
    /// Vendor-specific capability ID (used by VirtIO PCI).
    pub const CAP_ID_VENDOR: u8 = 0x09;
}
