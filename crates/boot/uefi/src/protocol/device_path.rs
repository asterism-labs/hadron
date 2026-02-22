//! UEFI Device Path Protocol.
//!
//! The Device Path Protocol defines the programmatic path to a device. Device paths are used
//! to locate devices and load images from them.

/// The Device Path Protocol structure.
///
/// A device path is a variable-length binary structure made up of variable-length nodes.
/// Each node has a type, sub-type, and length. The end of a device path is marked by
/// an end-of-device-path node.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DevicePathProtocol {
    /// The type of device path node.
    pub node_type: u8,
    /// The sub-type of the device path node.
    pub sub_type: u8,
    /// The length of this structure in bytes, stored as two bytes (little-endian).
    pub length: [u8; 2],
}

impl DevicePathProtocol {
    /// Returns the total length of this device path node in bytes.
    #[must_use]
    pub const fn node_length(&self) -> u16 {
        u16::from_le_bytes(self.length)
    }

    /// Returns `true` if this node marks the end of the entire device path.
    #[must_use]
    pub const fn is_end(&self) -> bool {
        self.node_type == node_type::END && self.sub_type == 0xFF
    }
}

/// Device path node type constants.
pub mod node_type {
    /// Hardware Device Path.
    pub const HARDWARE: u8 = 0x01;
    /// ACPI Device Path.
    pub const ACPI: u8 = 0x02;
    /// Messaging Device Path.
    pub const MESSAGING: u8 = 0x03;
    /// Media Device Path.
    pub const MEDIA: u8 = 0x04;
    /// BIOS Boot Specification Device Path.
    pub const BIOS_BOOT_SPEC: u8 = 0x05;
    /// End of Hardware Device Path.
    pub const END: u8 = 0x7F;
}

// ── Compile-time layout assertions ──────────────────────────────────

const _: () = assert!(core::mem::size_of::<DevicePathProtocol>() == 4);
