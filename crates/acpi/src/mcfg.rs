//! PCI Express Memory-Mapped Configuration (MCFG) table parsing.
//!
//! The MCFG table describes the PCI Express Enhanced Configuration Access
//! Mechanism (ECAM) base addresses for each PCI segment group.

use hadron_binparse::FromBytes;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// MCFG table signature.
pub const MCFG_SIGNATURE: &[u8; 4] = b"MCFG";

/// A single MCFG configuration space entry.
///
/// Each entry describes the ECAM base address for a PCI segment group and
/// the range of bus numbers it covers.
#[derive(Debug, Clone, Copy, FromBytes)]
#[repr(C, packed)]
pub struct McfgEntry {
    /// Base physical address of the enhanced configuration mechanism.
    pub base_address: u64,
    /// PCI segment group number.
    pub segment_group: u16,
    /// Start PCI bus number decoded by this entry.
    pub start_bus: u8,
    /// End PCI bus number decoded by this entry.
    pub end_bus: u8,
    /// Reserved.
    _reserved: u32,
}

impl McfgEntry {
    /// Size of a single MCFG entry in bytes.
    pub const SIZE: usize = 16;
}

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
        // Map the SDT header.
        // SAFETY: caller provides a valid physical address.
        let header_data = unsafe { handler.map_physical_region(phys, SdtHeader::SIZE) };
        let header = SdtHeader::read_from_bytes(header_data).ok_or(AcpiError::TruncatedData)?;

        if &header.signature() != MCFG_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }

        let total_len = header.length() as usize;

        // Map the entire table.
        // SAFETY: phys is valid, total_len comes from the header.
        let table_data = unsafe { handler.map_physical_region(phys, total_len) };

        // Validate checksum.
        if !crate::sdt::validate_checksum(table_data) {
            return Err(AcpiError::InvalidChecksum);
        }

        let entries_offset = SdtHeader::SIZE + Self::RESERVED_SIZE;
        let entries_data = table_data.get(entries_offset..).unwrap_or(&[]);
        let entry_count = entries_data.len() / McfgEntry::SIZE;

        Ok(Self {
            entries_data,
            entry_count,
        })
    }

    /// Returns an iterator over the MCFG configuration space entries.
    #[must_use]
    pub fn entries(&self) -> McfgEntryIter {
        McfgEntryIter {
            data: self.entries_data,
            pos: 0,
            remaining: self.entry_count,
        }
    }

    /// Returns the number of MCFG entries.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entry_count
    }
}

/// Iterator over MCFG configuration space entries.
pub struct McfgEntryIter {
    /// Byte slice covering the entry data.
    data: &'static [u8],
    /// Current byte offset.
    pos: usize,
    /// Number of entries remaining.
    remaining: usize,
}

impl Iterator for McfgEntryIter {
    type Item = McfgEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;

        let entry = McfgEntry::read_at(self.data, self.pos)?;
        self.pos += McfgEntry::SIZE;
        Some(entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for McfgEntryIter {}
