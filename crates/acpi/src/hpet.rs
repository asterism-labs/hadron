//! High Precision Event Timer (HPET) table parsing.
//!
//! The HPET table provides the base address and configuration parameters of
//! the HPET hardware timer, which offers a higher-resolution alternative to
//! the legacy 8254 PIT.

use core::ptr;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// HPET table signature.
pub const HPET_SIGNATURE: &[u8; 4] = b"HPET";

/// Generic Address Structure used to describe the HPET base address.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct GenericAddress {
    /// Address space ID (0 = system memory, 1 = system I/O).
    pub address_space_id: u8,
    /// Register bit width.
    pub register_bit_width: u8,
    /// Register bit offset.
    pub register_bit_offset: u8,
    /// Reserved / access size.
    pub reserved: u8,
    /// Address within the given address space.
    pub address: u64,
}

/// Raw HPET table fields following the SDT header.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
struct HpetRaw {
    event_timer_block_id: u32,
    base_address: GenericAddress,
    hpet_number: u16,
    minimum_tick: u16,
    page_protection: u8,
}

/// Parsed HPET table.
#[derive(Debug, Clone, Copy)]
pub struct HpetTable {
    /// Hardware ID of the event timer block.
    pub event_timer_block_id: u32,
    /// Base address of the HPET register block.
    pub base_address: GenericAddress,
    /// HPET sequence number (used when multiple HPETs are present).
    pub hpet_number: u16,
    /// Minimum clock tick in periodic mode, in femtoseconds.
    pub minimum_tick: u16,
    /// Page protection and OEM attribute.
    pub page_protection: u8,
}

impl HpetTable {
    /// Parse an HPET table from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `HPET`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        // Map the SDT header to learn the total table length.
        // SAFETY: caller provides a valid physical address.
        let header_ptr = unsafe { handler.map_physical_region(phys, SdtHeader::SIZE) };
        // SAFETY: header_ptr is valid for SdtHeader::SIZE bytes.
        let header = unsafe { SdtHeader::read_from(header_ptr) };

        if &header.signature() != HPET_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }

        let total_len = header.length() as usize;

        // Map the entire table.
        // SAFETY: phys is valid, total_len comes from the header.
        let table_ptr = unsafe { handler.map_physical_region(phys, total_len) };

        // Validate checksum.
        // SAFETY: table_ptr is valid for total_len bytes.
        if !unsafe { crate::sdt::validate_checksum(table_ptr, total_len) } {
            return Err(AcpiError::InvalidChecksum);
        }

        // Read the HPET-specific fields after the SDT header.
        // SAFETY: the table is large enough for the header + HPET fields.
        let raw: HpetRaw =
            unsafe { ptr::read_unaligned(table_ptr.add(SdtHeader::SIZE).cast()) };

        Ok(Self {
            event_timer_block_id: raw.event_timer_block_id,
            base_address: raw.base_address,
            hpet_number: raw.hpet_number,
            minimum_tick: raw.minimum_tick,
            page_protection: raw.page_protection,
        })
    }
}
