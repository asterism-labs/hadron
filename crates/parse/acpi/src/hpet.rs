//! High Precision Event Timer (HPET) table parsing.
//!
//! The HPET table provides the base address and configuration parameters of
//! the HPET hardware timer, which offers a higher-resolution alternative to
//! the legacy 8254 PIT.

use hadron_binparse::FromBytes;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// HPET table signature.
pub const HPET_SIGNATURE: &[u8; 4] = b"HPET";

/// Generic Address Structure used to describe the HPET base address.
#[derive(Debug, Clone, Copy, FromBytes)]
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
#[derive(Debug, Clone, Copy, FromBytes)]
#[repr(C, packed)]
struct HpetRaw {
    event_timer_block_id: u32,
    base_address: GenericAddress,
    hpet_number: u8,
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
    pub hpet_number: u8,
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
        let table = crate::sdt::load_table(handler, phys, HPET_SIGNATURE)?;

        let raw = HpetRaw::read_at(table.data, SdtHeader::SIZE).ok_or(AcpiError::TruncatedData)?;

        Ok(Self {
            event_timer_block_id: raw.event_timer_block_id,
            base_address: raw.base_address,
            hpet_number: raw.hpet_number,
            minimum_tick: raw.minimum_tick,
            page_protection: raw.page_protection,
        })
    }
}
