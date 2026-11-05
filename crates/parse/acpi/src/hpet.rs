//! High Precision Event Timer (HPET) table parsing.
//!
//! The HPET table provides the base address and configuration parameters of
//! the HPET hardware timer, which offers a higher-resolution alternative to
//! the legacy 8254 PIT.
//!
//! # References
//!
//! - ACPI Specification 6.5, §5.2.28: HPET Description Table
//!   <https://uefi.org/specs/ACPI/6.5/>
//! - IA-PC HPET Specification 1.0a
//!   <https://www.intel.com/content/dam/www/public/us/en/documents/technical-specifications/software-developers-hpet-spec-1-0a.pdf>

use zerocopy::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// HPET table signature.
pub const HPET_SIGNATURE: &[u8; 4] = b"HPET";

/// Generic Address Structure used to describe the HPET base address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Generic Address Structure as laid out in the table.
#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RawGenericAddress {
    address_space_id: u8,
    register_bit_width: u8,
    register_bit_offset: u8,
    reserved: u8,
    address: U64,
}

/// Raw HPET table fields following the SDT header.
#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct HpetRaw {
    event_timer_block_id: U32,
    base_address: RawGenericAddress,
    hpet_number: u8,
    minimum_tick: U16,
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

        let raw: HpetRaw =
            crate::read::at(table.data, SdtHeader::SIZE).ok_or(AcpiError::TruncatedData)?;
        let gas = raw.base_address;

        Ok(Self {
            event_timer_block_id: raw.event_timer_block_id.get(),
            base_address: GenericAddress {
                address_space_id: gas.address_space_id,
                register_bit_width: gas.register_bit_width,
                register_bit_offset: gas.register_bit_offset,
                reserved: gas.reserved,
                address: gas.address.get(),
            },
            hpet_number: raw.hpet_number,
            minimum_tick: raw.minimum_tick.get(),
            page_protection: raw.page_protection,
        })
    }
}
