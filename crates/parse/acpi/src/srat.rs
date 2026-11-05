//! System Resource Affinity Table (SRAT) parsing.
//!
//! The SRAT describes NUMA topology by mapping processors and memory regions
//! to proximity domains (NUMA nodes).
//!
//! # References
//!
//! - ACPI Specification 6.5, §5.2.16: System Resource Affinity Table (SRAT)
//!   <https://uefi.org/specs/ACPI/6.5/>

use crate::read::{split_u8_entry, u8_at, u32_at, u64_at};
use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// SRAT table signature.
pub const SRAT_SIGNATURE: &[u8; 4] = b"SRAT";

/// Parsed SRAT table.
///
/// The entry data is accessed through the [`SratEntryIter`] iterator returned
/// by [`Srat::entries`].
pub struct Srat {
    /// Byte slice covering the entry data after the fixed header.
    entries_data: &'static [u8],
}

impl Srat {
    /// Size of the fixed SRAT fields after the SDT header.
    ///
    /// 4 bytes reserved + 8 bytes reserved = 12 bytes.
    const FIELDS_SIZE: usize = 12;

    /// Parse a SRAT from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `SRAT`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, SRAT_SIGNATURE)?;

        let entries_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let entries_data = table.data.get(entries_offset..).unwrap_or(&[]);

        Ok(Self { entries_data })
    }

    /// Returns an iterator over the SRAT affinity entries.
    #[must_use]
    pub fn entries(&self) -> SratEntryIter<'_> {
        SratEntryIter {
            data: self.entries_data,
        }
    }
}

/// A single SRAT affinity structure entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SratEntry {
    /// Type 0: Processor Local APIC/SAPIC Affinity.
    ProcessorLocalApicAffinity {
        /// Low byte of the proximity domain.
        proximity_domain_lo: u8,
        /// Local APIC ID.
        apic_id: u8,
        /// Flags (bit 0: enabled).
        flags: u32,
        /// Local SAPIC EID.
        sapic_eid: u8,
        /// High 3 bytes of the proximity domain.
        proximity_domain_hi: [u8; 3],
        /// Clock domain.
        clock_domain: u32,
    },

    /// Type 1: Memory Affinity.
    MemoryAffinity {
        /// Proximity domain.
        proximity_domain: u32,
        /// Base address of the memory range.
        base_address: u64,
        /// Length of the memory range in bytes.
        length: u64,
        /// Flags (bit 0: enabled, bit 1: hot-pluggable, bit 2: non-volatile).
        flags: u32,
    },

    /// Type 2: Processor Local x2APIC Affinity.
    X2ApicAffinity {
        /// Proximity domain.
        proximity_domain: u32,
        /// Processor's local x2APIC ID.
        x2apic_id: u32,
        /// Flags (bit 0: enabled).
        flags: u32,
        /// Clock domain.
        clock_domain: u32,
    },

    /// An entry type that we do not parse, or a known type that is shorter
    /// than its fixed size.
    Unknown {
        /// The entry type byte.
        entry_type: u8,
        /// The entry length including the 2-byte header.
        length: u8,
    },
}

impl SratEntry {
    /// Decodes one entry, header included.
    ///
    /// Returns `None` when `entry` is shorter than the fixed size of its
    /// type; the caller reports that entry as [`SratEntry::Unknown`].
    fn decode(entry_type: u8, entry: &[u8]) -> Option<Self> {
        let min_length = match entry_type {
            0 => 16,
            1 => 40,
            2 => 24,
            _ => return None,
        };
        if entry.len() < min_length {
            return None;
        }
        Some(match entry_type {
            0 => Self::ProcessorLocalApicAffinity {
                proximity_domain_lo: u8_at(entry, 2)?,
                apic_id: u8_at(entry, 3)?,
                flags: u32_at(entry, 4)?,
                sapic_eid: u8_at(entry, 8)?,
                proximity_domain_hi: crate::read::at(entry, 9)?,
                clock_domain: u32_at(entry, 12)?,
            },
            1 => Self::MemoryAffinity {
                proximity_domain: u32_at(entry, 2)?,
                base_address: u64_at(entry, 8)?,
                length: u64_at(entry, 16)?,
                flags: u32_at(entry, 28)?,
            },
            _ => Self::X2ApicAffinity {
                proximity_domain: u32_at(entry, 4)?,
                x2apic_id: u32_at(entry, 8)?,
                flags: u32_at(entry, 12)?,
                clock_domain: u32_at(entry, 16)?,
            },
        })
    }
}

/// Iterator over SRAT affinity entries.
///
/// Iteration ends at the first entry whose length is below 2 or runs past
/// the end of the table.
pub struct SratEntryIter<'a> {
    data: &'a [u8],
}

impl Iterator for SratEntryIter<'_> {
    type Item = SratEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = split_u8_entry(&mut self.data)?;
        let (entry_type, length) = (entry[0], entry[1]);
        Some(
            SratEntry::decode(entry_type, entry)
                .unwrap_or(SratEntry::Unknown { entry_type, length }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_memory_affinity_and_stops_on_truncation() {
        let mut data = [0u8; 40 + 3];
        data[0] = 1;
        data[1] = 40;
        data[2..6].copy_from_slice(&3u32.to_le_bytes());
        data[8..16].copy_from_slice(&0x1_0000_0000u64.to_le_bytes());
        data[16..24].copy_from_slice(&0x4000_0000u64.to_le_bytes());
        data[28..32].copy_from_slice(&1u32.to_le_bytes());
        // A trailing entry that claims more bytes than remain.
        data[40] = 2;
        data[41] = 24;

        let mut iter = SratEntryIter { data: &data };
        assert_eq!(
            iter.next(),
            Some(SratEntry::MemoryAffinity {
                proximity_domain: 3,
                base_address: 0x1_0000_0000,
                length: 0x4000_0000,
                flags: 1,
            })
        );
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn short_known_entry_is_unknown() {
        // A processor affinity entry that claims 8 bytes instead of 16.
        let data = [0, 8, 0, 0, 0, 0, 0, 0];
        let mut iter = SratEntryIter { data: &data };
        assert_eq!(
            iter.next(),
            Some(SratEntry::Unknown {
                entry_type: 0,
                length: 8
            })
        );
        assert_eq!(iter.next(), None);
    }
}
