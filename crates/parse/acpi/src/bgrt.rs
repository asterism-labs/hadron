//! Boot Graphics Resource Table (BGRT) parsing.
//!
//! The BGRT describes the boot-time logo image displayed by firmware,
//! including its type, physical address, and screen coordinates.

use hadron_binparse::FromBytes;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// BGRT table signature.
pub const BGRT_SIGNATURE: &[u8; 4] = b"BGRT";

/// Raw BGRT table fields following the SDT header.
#[derive(Debug, Clone, Copy, FromBytes)]
#[repr(C, packed)]
struct BgrtRaw {
    version: u16,
    status: u8,
    image_type: u8,
    image_address: u64,
    image_offset_x: u32,
    image_offset_y: u32,
}

/// Parsed BGRT table.
#[derive(Debug, Clone, Copy)]
pub struct BgrtTable {
    /// BGRT version (must be 1).
    pub version: u16,
    /// Status field (bit 0: displayed, bits 1-2: orientation).
    pub status: u8,
    /// Image type (0 = BMP).
    pub image_type: u8,
    /// Physical address of the boot image in memory.
    pub image_address: u64,
    /// X offset of the image on screen.
    pub image_offset_x: u32,
    /// Y offset of the image on screen.
    pub image_offset_y: u32,
}

impl BgrtTable {
    /// Parse a BGRT table from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `BGRT`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, BGRT_SIGNATURE)?;

        let raw = BgrtRaw::read_at(table.data, SdtHeader::SIZE).ok_or(AcpiError::TruncatedData)?;

        Ok(Self {
            version: raw.version,
            status: raw.status,
            image_type: raw.image_type,
            image_address: raw.image_address,
            image_offset_x: raw.image_offset_x,
            image_offset_y: raw.image_offset_y,
        })
    }
}
