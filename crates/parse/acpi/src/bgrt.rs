//! Boot Graphics Resource Table (BGRT) parsing.
//!
//! The BGRT describes the boot-time logo image displayed by firmware,
//! including its type, physical address, and screen coordinates.
//!
//! # References
//!
//! - ACPI Specification 6.5, §5.2.22: Boot Graphics Resource Table (BGRT)
//!   <https://uefi.org/specs/ACPI/6.5/>

use zerocopy::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// BGRT table signature.
pub const BGRT_SIGNATURE: &[u8; 4] = b"BGRT";

/// Raw BGRT table fields following the SDT header.
#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct BgrtRaw {
    version: U16,
    status: u8,
    image_type: u8,
    image_address: U64,
    image_offset_x: U32,
    image_offset_y: U32,
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

        let raw: BgrtRaw =
            crate::read::at(table.data, SdtHeader::SIZE).ok_or(AcpiError::TruncatedData)?;

        Ok(Self {
            version: raw.version.get(),
            status: raw.status,
            image_type: raw.image_type,
            image_address: raw.image_address.get(),
            image_offset_x: raw.image_offset_x.get(),
            image_offset_y: raw.image_offset_y.get(),
        })
    }
}
