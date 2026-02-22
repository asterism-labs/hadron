//! System Resource Affinity Table (SRAT) parsing.
//!
//! The SRAT describes NUMA topology by mapping processors and memory regions
//! to proximity domains (NUMA nodes).

use hadron_binparse::TableEntries;

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
    pub fn entries(&self) -> SratEntryIter {
        SratEntry::iter(self.entries_data)
    }
}

/// A single SRAT affinity structure entry.
#[derive(Debug, Clone, Copy, TableEntries)]
#[table_entries(type_field = u8, length_field = u8)]
pub enum SratEntry {
    /// Type 0: Processor Local APIC/SAPIC Affinity.
    #[entry(type_id = 0, min_length = 16)]
    ProcessorLocalApicAffinity {
        /// Low byte of the proximity domain.
        #[field(offset = 2)]
        proximity_domain_lo: u8,
        /// Local APIC ID.
        #[field(offset = 3)]
        apic_id: u8,
        /// Flags (bit 0: enabled).
        #[field(offset = 4)]
        flags: u32,
        /// Local SAPIC EID.
        #[field(offset = 8)]
        sapic_eid: u8,
        /// High 3 bytes of the proximity domain.
        #[field(offset = 9)]
        proximity_domain_hi: [u8; 3],
        /// Clock domain.
        #[field(offset = 12)]
        clock_domain: u32,
    },

    /// Type 1: Memory Affinity.
    #[entry(type_id = 1, min_length = 40)]
    MemoryAffinity {
        /// Proximity domain.
        #[field(offset = 2)]
        proximity_domain: u32,
        /// Base address of the memory range.
        #[field(offset = 8)]
        base_address: u64,
        /// Length of the memory range in bytes.
        #[field(offset = 16)]
        length: u64,
        /// Flags (bit 0: enabled, bit 1: hot-pluggable, bit 2: non-volatile).
        #[field(offset = 28)]
        flags: u32,
    },

    /// Type 2: Processor Local x2APIC Affinity.
    #[entry(type_id = 2, min_length = 24)]
    X2ApicAffinity {
        /// Proximity domain.
        #[field(offset = 4)]
        proximity_domain: u32,
        /// Processor's local x2APIC ID.
        #[field(offset = 8)]
        x2apic_id: u32,
        /// Flags (bit 0: enabled).
        #[field(offset = 12)]
        flags: u32,
        /// Clock domain.
        #[field(offset = 16)]
        clock_domain: u32,
    },

    /// An entry type that we do not parse.
    #[fallback]
    Unknown {
        /// The entry type byte.
        entry_type: u8,
        /// The entry length including the 2-byte header.
        length: u8,
    },
}
