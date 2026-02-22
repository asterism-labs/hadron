//! UEFI Globally Unique Identifier (GUID) type and well-known constants.
//!
//! This module provides the [`EfiGuid`] type, a 128-bit identifier used extensively throughout
//! the UEFI specification to identify protocols, tables, and other entities.

use core::fmt;

/// A UEFI Globally Unique Identifier (GUID).
///
/// GUIDs are 128-bit identifiers formatted as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
/// They are used throughout UEFI to uniquely identify protocols, configuration tables,
/// and other entities.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EfiGuid {
    /// The first 32 bits of the GUID.
    pub data1: u32,
    /// The next 16 bits of the GUID.
    pub data2: u16,
    /// The next 16 bits of the GUID.
    pub data3: u16,
    /// The remaining 64 bits of the GUID.
    pub data4: [u8; 8],
}

#[expect(
    clippy::unreadable_literal,
    reason = "GUID bytes are inherently opaque"
)]
impl EfiGuid {
    /// Creates a new GUID from its component parts.
    #[must_use]
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        Self {
            data1,
            data2,
            data3,
            data4,
        }
    }

    // ── Protocol GUIDs ───────────────────────────────────────────────

    /// Graphics Output Protocol GUID.
    pub const GRAPHICS_OUTPUT_PROTOCOL: Self = Self::new(
        0x9042a9de,
        0x23dc,
        0x4a38,
        [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
    );

    /// Simple Text Input Protocol GUID.
    pub const SIMPLE_TEXT_INPUT_PROTOCOL: Self = Self::new(
        0x387477c1,
        0x69c7,
        0x11d2,
        [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    /// Simple Text Output Protocol GUID.
    pub const SIMPLE_TEXT_OUTPUT_PROTOCOL: Self = Self::new(
        0x387477c2,
        0x69c7,
        0x11d2,
        [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    /// Simple File System Protocol GUID.
    pub const SIMPLE_FILE_SYSTEM_PROTOCOL: Self = Self::new(
        0x0964e5b22,
        0x6459,
        0x11d2,
        [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    /// Loaded Image Protocol GUID.
    pub const LOADED_IMAGE_PROTOCOL: Self = Self::new(
        0x5b1b31a1,
        0x9562,
        0x11d2,
        [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    /// Device Path Protocol GUID.
    pub const DEVICE_PATH_PROTOCOL: Self = Self::new(
        0x09576e91,
        0x6d3f,
        0x11d2,
        [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    /// Block I/O Protocol GUID.
    pub const BLOCK_IO_PROTOCOL: Self = Self::new(
        0x0964e5b21,
        0x6459,
        0x11d2,
        [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    // ── File Information GUIDs ───────────────────────────────────────

    /// File Info GUID (used with `GetInfo`/`SetInfo`).
    pub const FILE_INFO: Self = Self::new(
        0x09576e92,
        0x6d3f,
        0x11d2,
        [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    /// File System Info GUID.
    pub const FILE_SYSTEM_INFO: Self = Self::new(
        0x09576e93,
        0x6d3f,
        0x11d2,
        [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
    );

    /// File System Volume Label Info GUID.
    pub const FILE_SYSTEM_VOLUME_LABEL_INFO: Self = Self::new(
        0xdb47d7d3,
        0xfe81,
        0x11d3,
        [0x9a, 0x35, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d],
    );

    // ── Configuration Table GUIDs ────────────────────────────────────

    /// ACPI 2.0 Table GUID.
    pub const ACPI_20_TABLE: Self = Self::new(
        0x8868e871,
        0xe4f1,
        0x11d3,
        [0xbc, 0x22, 0x00, 0x80, 0xc7, 0x3c, 0x88, 0x81],
    );

    /// ACPI 1.0 Table GUID.
    pub const ACPI_TABLE: Self = Self::new(
        0xeb9d2d30,
        0x2d88,
        0x11d3,
        [0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d],
    );

    /// SMBIOS Table GUID.
    pub const SMBIOS_TABLE: Self = Self::new(
        0xeb9d2d31,
        0x2d88,
        0x11d3,
        [0x9a, 0x16, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0x4d],
    );

    /// SMBIOS 3.0 Table GUID.
    pub const SMBIOS3_TABLE: Self = Self::new(
        0xf2fd1544,
        0x9794,
        0x4a2c,
        [0x99, 0x2e, 0xe5, 0xbb, 0xcf, 0x20, 0xe3, 0x94],
    );

    /// Device Tree Table GUID.
    pub const DEVICE_TREE_TABLE: Self = Self::new(
        0xb1b621d5,
        0xf19c,
        0x41a5,
        [0x83, 0x0b, 0xd9, 0x15, 0x2c, 0x69, 0xaa, 0xe0],
    );
}

impl fmt::Debug for EfiGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EfiGuid({self})")
    }
}

impl fmt::Display for EfiGuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7],
        )
    }
}

// ── Compile-time layout assertions ──────────────────────────────────

const _: () = assert!(core::mem::size_of::<EfiGuid>() == 16);
