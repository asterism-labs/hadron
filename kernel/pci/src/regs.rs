//! Standard PCI configuration space register offsets and constants.

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

// -- Status register bits -----------------------------------------------------

/// Bit 4 of the Status register: capabilities list present.
pub const STATUS_CAPABILITIES_LIST: u16 = 1 << 4;

// -- PCI capability IDs -------------------------------------------------------

/// MSI-X capability ID.
pub const CAP_ID_MSIX: u8 = 0x11;
/// Vendor-specific capability ID (used by VirtIO PCI).
pub const CAP_ID_VENDOR: u8 = 0x09;
