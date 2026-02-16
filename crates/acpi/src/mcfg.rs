//! PCI Express Memory-Mapped Configuration (MCFG) table parsing.
//!
//! The MCFG table describes the PCI Express Enhanced Configuration Access
//! Mechanism (ECAM) base addresses for each PCI segment group.

use core::ptr;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// MCFG table signature.
pub const MCFG_SIGNATURE: &[u8; 4] = b"MCFG";

/// A single MCFG configuration space entry.
///
/// Each entry describes the ECAM base address for a PCI segment group and
/// the range of bus numbers it covers.
#[derive(Debug, Clone, Copy)]
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
    /// Pointer to the first [`McfgEntry`] in the mapped table.
    entries_ptr: *const u8,
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
        let header_ptr = unsafe { handler.map_physical_region(phys, SdtHeader::SIZE) };
        // SAFETY: header_ptr is valid for SdtHeader::SIZE bytes.
        let header = unsafe { SdtHeader::read_from(header_ptr) };

        if &header.signature() != MCFG_SIGNATURE {
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

        let entries_offset = SdtHeader::SIZE + Self::RESERVED_SIZE;
        let entries_len = total_len.saturating_sub(entries_offset);
        let entry_count = entries_len / McfgEntry::SIZE;

        // SAFETY: entries_offset is within the mapped region.
        let entries_ptr = unsafe { table_ptr.add(entries_offset) };

        Ok(Self {
            entries_ptr,
            entry_count,
        })
    }

    /// Returns an iterator over the MCFG configuration space entries.
    #[must_use]
    pub fn entries(&self) -> McfgEntryIter {
        McfgEntryIter {
            ptr: self.entries_ptr,
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
    /// Pointer to the current entry.
    ptr: *const u8,
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

        // SAFETY: the MCFG parser ensures the pointer region is valid for
        // entry_count * McfgEntry::SIZE bytes.
        let entry = unsafe { ptr::read_unaligned(self.ptr.cast::<McfgEntry>()) };
        // SAFETY: advancing within the valid entry region.
        self.ptr = unsafe { self.ptr.add(McfgEntry::SIZE) };
        Some(entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for McfgEntryIter {}
