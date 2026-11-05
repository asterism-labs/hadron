//! PCI Express Memory-Mapped Configuration (MCFG) table parsing.
//!
//! The MCFG table describes the PCI Express Enhanced Configuration Access
//! Mechanism (ECAM) base addresses for each PCI segment group.
//!
//! # References
//!
//! - PCI Firmware Specification 3.0, §4.1.2: MCFG Table
//!   <https://pcisig.com/specifications>
//! - `OSDev` Wiki: PCI Express
//!   <https://wiki.osdev.org/PCI_Express>

use zerocopy::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// MCFG table signature.
pub const MCFG_SIGNATURE: &[u8; 4] = b"MCFG";

/// A single MCFG configuration space entry.
///
/// Each entry describes the ECAM base address for a PCI segment group and
/// the range of bus numbers it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McfgEntry {
    /// Base physical address of the enhanced configuration mechanism.
    pub base_address: u64,
    /// PCI segment group number.
    pub segment_group: u16,
    /// Start PCI bus number decoded by this entry.
    pub start_bus: u8,
    /// End PCI bus number decoded by this entry.
    pub end_bus: u8,
}

impl McfgEntry {
    /// Size of a single MCFG entry in bytes.
    pub const SIZE: usize = 16;
}

/// MCFG entry as laid out in the table.
#[derive(FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
struct RawMcfgEntry {
    base_address: U64,
    segment_group: U16,
    start_bus: u8,
    end_bus: u8,
    _reserved: U32,
}

const _: () = assert!(size_of::<RawMcfgEntry>() == McfgEntry::SIZE);

/// Parsed MCFG table.
pub struct Mcfg {
    /// Byte slice covering the entry data.
    entries_data: &'static [u8],
    /// Number of entries.
    entry_count: usize,
}

impl Mcfg {
    /// Size of the reserved field between the SDT header and the entries.
    const RESERVED_SIZE: usize = 8;

    /// Parse an MCFG table from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `MCFG`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, MCFG_SIGNATURE)?;

        let entries_offset = SdtHeader::SIZE + Self::RESERVED_SIZE;
        let entries_data = table.data.get(entries_offset..).unwrap_or(&[]);
        let entry_count = entries_data.len() / McfgEntry::SIZE;

        Ok(Self {
            entries_data,
            entry_count,
        })
    }

    /// Returns an iterator over the MCFG configuration space entries.
    #[must_use]
    pub fn entries(&self) -> McfgEntryIter<'_> {
        McfgEntryIter {
            data: self.entries_data,
        }
    }

    /// Returns the number of MCFG entries.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }
}

/// Iterator over MCFG configuration space entries.
///
/// Trailing bytes too short for a whole entry are ignored.
pub struct McfgEntryIter<'a> {
    data: &'a [u8],
}

impl Iterator for McfgEntryIter<'_> {
    type Item = McfgEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let (raw, rest) = RawMcfgEntry::read_from_prefix(self.data).ok()?;
        self.data = rest;
        Some(McfgEntry {
            base_address: raw.base_address.get(),
            segment_group: raw.segment_group.get(),
            start_bus: raw.start_bus,
            end_bus: raw.end_bus,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.data.len() / McfgEntry::SIZE;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for McfgEntryIter<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iterates_whole_entries_only() {
        let mut data = [0u8; 2 * McfgEntry::SIZE + 5];
        data[..8].copy_from_slice(&0xE000_0000u64.to_le_bytes());
        data[10] = 0;
        data[11] = 0xFF;
        data[16..24].copy_from_slice(&0xF000_0000u64.to_le_bytes());
        data[24..26].copy_from_slice(&1u16.to_le_bytes());

        let iter = McfgEntryIter { data: &data };
        assert_eq!(iter.len(), 2);
        let entries: [McfgEntry; 2] = {
            let mut iter = iter;
            [iter.next().unwrap(), iter.next().unwrap()]
        };
        assert_eq!(
            entries[0],
            McfgEntry {
                base_address: 0xE000_0000,
                segment_group: 0,
                start_bus: 0,
                end_bus: 0xFF
            }
        );
        assert_eq!(entries[1].segment_group, 1);
        assert_eq!(McfgEntryIter { data: &data[32..] }.next(), None);
    }
}
