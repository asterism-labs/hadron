//! Multiple APIC Description Table (MADT) parsing.
//!
//! The MADT describes the interrupt controller topology of the system,
//! including local APICs, I/O APICs, interrupt source overrides, and NMI
//! sources.

use hadron_binparse::{FromBytes, TableEntries};

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// MADT table signature (`b"APIC"`).
pub const MADT_SIGNATURE: &[u8; 4] = b"APIC";

/// Raw MADT header fields that follow the SDT header.
#[derive(Debug, Clone, Copy, FromBytes)]
#[repr(C, packed)]
struct MadtHeaderFields {
    /// Physical address of the local APIC.
    local_apic_address: u32,
    /// MADT flags (bit 0: `PCAT_COMPAT`).
    flags: u32,
}

/// Parsed MADT table.
///
/// The entry data is accessed through the [`MadtEntryIter`] iterator returned
/// by [`Madt::entries`].
pub struct Madt {
    /// Physical address of the local APIC.
    pub local_apic_address: u32,
    /// MADT flags (bit 0: dual 8259 PICs installed).
    pub flags: u32,
    /// Byte slice covering the entry data.
    entries_data: &'static [u8],
}

impl Madt {
    /// Size of the fixed MADT fields after the SDT header (address + flags).
    const FIELDS_SIZE: usize = 8;

    /// Parse a MADT from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidChecksum`] if the table checksum is invalid,
    /// or [`AcpiError::InvalidSignature`] if the table signature is not `APIC`.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, MADT_SIGNATURE)?;

        let fields = MadtHeaderFields::read_at(table.data, SdtHeader::SIZE)
            .ok_or(AcpiError::TruncatedData)?;

        let entries_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let entries_data = table.data.get(entries_offset..).unwrap_or(&[]);

        Ok(Self {
            local_apic_address: fields.local_apic_address,
            flags: fields.flags,
            entries_data,
        })
    }

    /// Returns an iterator over the MADT interrupt controller entries.
    #[must_use]
    pub fn entries(&self) -> MadtEntryIter {
        MadtEntry::iter(self.entries_data)
    }
}

/// A single MADT interrupt controller structure entry.
#[derive(Debug, Clone, Copy, TableEntries)]
#[table_entries(type_field = u8, length_field = u8)]
pub enum MadtEntry {
    /// Type 0: Processor Local APIC.
    #[entry(type_id = 0, min_length = 8)]
    LocalApic {
        /// ACPI processor UID.
        #[field(offset = 2)]
        acpi_processor_id: u8,
        /// The processor's local APIC ID.
        #[field(offset = 3)]
        apic_id: u8,
        /// Flags (bit 0: enabled, bit 1: online capable).
        #[field(offset = 4)]
        flags: u32,
    },

    /// Type 1: I/O APIC.
    #[entry(type_id = 1, min_length = 12)]
    IoApic {
        /// The I/O APIC ID.
        #[field(offset = 2)]
        io_apic_id: u8,
        /// Physical address of the I/O APIC registers.
        #[field(offset = 4)]
        io_apic_address: u32,
        /// Global System Interrupt base for this I/O APIC.
        #[field(offset = 8)]
        gsi_base: u32,
    },

    /// Type 2: Interrupt Source Override.
    #[entry(type_id = 2, min_length = 10)]
    InterruptSourceOverride {
        /// Constant: 0 (ISA bus).
        #[field(offset = 2)]
        bus: u8,
        /// ISA source IRQ number.
        #[field(offset = 3)]
        source: u8,
        /// Global System Interrupt number this source maps to.
        #[field(offset = 4)]
        gsi: u32,
        /// MPS INTI flags.
        #[field(offset = 8)]
        flags: u16,
    },

    /// Type 4: Non-Maskable Interrupt Source.
    #[entry(type_id = 4, min_length = 8)]
    NmiSource {
        /// MPS INTI flags.
        #[field(offset = 2)]
        flags: u16,
        /// Global System Interrupt number of the NMI source.
        #[field(offset = 4)]
        gsi: u32,
    },

    /// Type 5: Local APIC NMI.
    #[entry(type_id = 5, min_length = 6)]
    LocalApicNmi {
        /// ACPI processor UID (0xFF means all processors).
        #[field(offset = 2)]
        acpi_processor_id: u8,
        /// MPS INTI flags.
        #[field(offset = 3)]
        flags: u16,
        /// Local APIC LINT pin (0 or 1).
        #[field(offset = 5)]
        lint: u8,
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
