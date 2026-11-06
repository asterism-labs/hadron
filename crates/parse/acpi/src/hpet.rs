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
    /// The smallest period, in main counter ticks, that a comparator in
    /// periodic mode can be set to without losing interrupts.
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
        Self::read_fields(table.data).ok_or(AcpiError::TruncatedData)
    }

    /// Reads the fields from a whole table, header included. Returns
    /// `None` if the table is too short.
    fn read_fields(data: &[u8]) -> Option<Self> {
        let raw: HpetRaw = crate::read::at(data, SdtHeader::SIZE)?;
        let gas = raw.base_address;

        Some(Self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_fields_after_the_header() {
        // QEMU's q35 table: one 64-bit HPET with 3 comparators at
        // 0xFED0_0000.
        let mut data = [0u8; 56];
        data[36..40].copy_from_slice(&0x8086_A201u32.to_le_bytes());
        // Generic address: system memory, 64 bits wide, offset 0.
        data[40..44].copy_from_slice(&[0, 64, 0, 0]);
        data[44..52].copy_from_slice(&0xFED0_0000u64.to_le_bytes());
        data[52] = 0;
        data[53..55].copy_from_slice(&128u16.to_le_bytes());
        data[55] = 0;
        let hpet = HpetTable::read_fields(&data).expect("56 bytes is a whole table");
        assert_eq!(hpet.event_timer_block_id, 0x8086_A201);
        assert_eq!(hpet.base_address.address_space_id, 0);
        assert_eq!(hpet.base_address.register_bit_width, 64);
        assert_eq!(hpet.base_address.address, 0xFED0_0000);
        assert_eq!(hpet.hpet_number, 0);
        assert_eq!(hpet.minimum_tick, 128);
    }

    #[test]
    fn short_table_is_rejected() {
        assert!(HpetTable::read_fields(&[0u8; 55]).is_none());
    }
}
