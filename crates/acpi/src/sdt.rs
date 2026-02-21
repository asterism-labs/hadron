//! System Description Table (SDT) header and checksum utilities.

use hadron_binparse::FromBytes;

/// Standard ACPI System Description Table header.
///
/// This 36-byte header is present at the start of every ACPI table
/// (RSDT, XSDT, MADT, HPET, FADT, MCFG, etc.).
#[derive(Debug, Clone, Copy, FromBytes)]
#[repr(C, packed)]
pub struct SdtHeader {
    /// 4-byte ASCII signature identifying the table type.
    pub signature: [u8; 4],
    /// Total length of the table, including the header, in bytes.
    pub length: u32,
    /// Revision of the table structure.
    pub revision: u8,
    /// Checksum byte. The entire table, including the header, must sum to zero.
    pub checksum: u8,
    /// OEM-supplied identification string.
    pub oem_id: [u8; 6],
    /// OEM-supplied table identification string.
    pub oem_table_id: [u8; 8],
    /// OEM-supplied revision number.
    pub oem_revision: u32,
    /// Vendor ID of the utility that created the table.
    pub creator_id: u32,
    /// Revision of the utility that created the table.
    pub creator_revision: u32,
}

impl SdtHeader {
    /// The size of an SDT header in bytes.
    pub const SIZE: usize = 36;

    /// Read an [`SdtHeader`] from a byte slice.
    ///
    /// Returns `None` if the slice is shorter than [`SdtHeader::SIZE`] bytes.
    #[must_use]
    pub fn read_from_bytes(data: &[u8]) -> Option<Self> {
        Self::read_from(data)
    }

    /// Returns the 4-byte signature as a byte slice.
    #[must_use]
    pub fn signature(&self) -> [u8; 4] {
        self.signature
    }

    /// Returns the total length of this table (header included).
    #[must_use]
    pub fn length(&self) -> u32 {
        self.length
    }
}

/// Validate the checksum of a byte slice.
///
/// ACPI tables are designed so that the sum of all bytes in the table equals
/// zero (mod 256). This function computes that sum and returns `true` when
/// the checksum is valid.
#[must_use]
pub fn validate_checksum(data: &[u8]) -> bool {
    let mut sum: u8 = 0;
    for &byte in data {
        sum = sum.wrapping_add(byte);
    }
    sum == 0
}
