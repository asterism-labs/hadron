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

/// Mapped ACPI table data with a validated header.
///
/// Returned by [`load_table`] after performing the standard map-header,
/// verify-signature, map-full, validate-checksum sequence.
pub struct ValidatedTable {
    /// The validated SDT header.
    pub header: SdtHeader,
    /// The full table data (including header), checksum-validated.
    pub data: &'static [u8],
}

/// Maps and validates an ACPI table at the given physical address.
///
/// Performs the standard 4-step ACPI table loading sequence:
/// 1. Map the SDT header to learn the table length
/// 2. Verify the 4-byte signature matches `expected_signature`
/// 3. Map the full table
/// 4. Validate the checksum
///
/// # Errors
///
/// Returns [`AcpiError::TruncatedData`] if the header cannot be read,
/// [`AcpiError::InvalidSignature`] if the signature doesn't match,
/// or [`AcpiError::InvalidChecksum`] if the checksum fails.
pub fn load_table(
    handler: &impl super::AcpiHandler,
    phys: u64,
    expected_signature: &[u8; 4],
) -> Result<ValidatedTable, super::AcpiError> {
    // SAFETY: Caller provides a valid table physical address.
    let header_data = unsafe { handler.map_physical_region(phys, SdtHeader::SIZE) };
    let header = SdtHeader::read_from_bytes(header_data).ok_or(super::AcpiError::TruncatedData)?;

    if &header.signature() != expected_signature {
        return Err(super::AcpiError::InvalidSignature);
    }

    let total_len = header.length() as usize;

    // SAFETY: phys is valid, total_len comes from the validated header.
    let data = unsafe { handler.map_physical_region(phys, total_len) };

    if !validate_checksum(data) {
        return Err(super::AcpiError::InvalidChecksum);
    }

    Ok(ValidatedTable { header, data })
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
