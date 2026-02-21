//! Multiple APIC Description Table (MADT) parsing.
//!
//! The MADT describes the interrupt controller topology of the system,
//! including local APICs, I/O APICs, interrupt source overrides, and NMI
//! sources.

use hadron_binparse::FromBytes;

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
        // Map just the SDT header first to learn the total length.
        // SAFETY: caller provides a valid table physical address.
        let header_data = unsafe { handler.map_physical_region(phys, SdtHeader::SIZE) };
        let header = SdtHeader::read_from_bytes(header_data).ok_or(AcpiError::TruncatedData)?;

        if &header.signature() != MADT_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }

        let total_len = header.length() as usize;

        // Map the entire table.
        // SAFETY: phys is valid, total_len comes from the header.
        let table_data = unsafe { handler.map_physical_region(phys, total_len) };

        // Validate the checksum over the entire table.
        if !crate::sdt::validate_checksum(table_data) {
            return Err(AcpiError::InvalidChecksum);
        }

        // Read the fixed MADT fields after the SDT header.
        let fields = MadtHeaderFields::read_at(table_data, SdtHeader::SIZE)
            .ok_or(AcpiError::TruncatedData)?;

        let entries_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let entries_data = table_data
            .get(entries_offset..)
            .unwrap_or(&[]);

        Ok(Self {
            local_apic_address: fields.local_apic_address,
            flags: fields.flags,
            entries_data,
        })
    }

    /// Returns an iterator over the MADT interrupt controller entries.
    #[must_use]
    pub fn entries(&self) -> MadtEntryIter {
        MadtEntryIter {
            data: self.entries_data,
            pos: 0,
        }
    }
}

/// A single MADT interrupt controller structure entry.
#[derive(Debug, Clone, Copy)]
pub enum MadtEntry {
    /// Type 0: Processor Local APIC.
    LocalApic(LocalApic),
    /// Type 1: I/O APIC.
    IoApic(IoApic),
    /// Type 2: Interrupt Source Override.
    InterruptSourceOverride(InterruptSourceOverride),
    /// Type 4: Non-Maskable Interrupt (NMI) Source.
    NmiSource(NmiSource),
    /// Type 5: Local APIC NMI.
    LocalApicNmi(LocalApicNmi),
    /// An entry type that we do not parse.
    Unknown {
        /// The entry type byte.
        entry_type: u8,
        /// The entry length including the 2-byte header.
        length: u8,
    },
}

/// Processor Local APIC structure (MADT entry type 0).
#[derive(Debug, Clone, Copy)]
pub struct LocalApic {
    /// ACPI processor UID.
    pub acpi_processor_id: u8,
    /// The processor's local APIC ID.
    pub apic_id: u8,
    /// Flags (bit 0: enabled, bit 1: online capable).
    pub flags: u32,
}

/// I/O APIC structure (MADT entry type 1).
#[derive(Debug, Clone, Copy)]
pub struct IoApic {
    /// The I/O APIC ID.
    pub io_apic_id: u8,
    /// Physical address of the I/O APIC registers.
    pub io_apic_address: u32,
    /// Global System Interrupt base for this I/O APIC.
    pub gsi_base: u32,
}

/// Interrupt Source Override (MADT entry type 2).
#[derive(Debug, Clone, Copy)]
pub struct InterruptSourceOverride {
    /// Constant: 0 (ISA bus).
    pub bus: u8,
    /// ISA source IRQ number.
    pub source: u8,
    /// Global System Interrupt number this source maps to.
    pub gsi: u32,
    /// MPS INTI flags.
    pub flags: u16,
}

/// Non-Maskable Interrupt Source (MADT entry type 4).
#[derive(Debug, Clone, Copy)]
pub struct NmiSource {
    /// MPS INTI flags.
    pub flags: u16,
    /// Global System Interrupt number of the NMI source.
    pub gsi: u32,
}

/// Local APIC NMI structure (MADT entry type 5).
#[derive(Debug, Clone, Copy)]
pub struct LocalApicNmi {
    /// ACPI processor UID (0xFF means all processors).
    pub acpi_processor_id: u8,
    /// MPS INTI flags.
    pub flags: u16,
    /// Local APIC LINT pin (0 or 1).
    pub lint: u8,
}

/// Iterator over MADT interrupt controller structure entries.
pub struct MadtEntryIter {
    /// Byte slice covering the entry data.
    data: &'static [u8],
    /// Current position within `data`.
    pos: usize,
}

impl Iterator for MadtEntryIter {
    type Item = MadtEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let remaining = &self.data[self.pos..];

        // Each entry has at least a 2-byte header: type (u8) + length (u8).
        if remaining.len() < 2 {
            return None;
        }

        let entry_type = remaining[0];
        let length = remaining[1] as usize;

        if length < 2 || length > remaining.len() {
            return None;
        }

        let entry_data = &remaining[..length];

        let entry = match entry_type {
            // Type 0: Local APIC — 8 bytes total.
            0 if length >= 8 => MadtEntry::LocalApic(LocalApic {
                acpi_processor_id: entry_data[2],
                apic_id: entry_data[3],
                flags: u32::read_at(entry_data, 4).unwrap_or(0),
            }),

            // Type 1: I/O APIC — 12 bytes total.
            1 if length >= 12 => MadtEntry::IoApic(IoApic {
                io_apic_id: entry_data[2],
                // byte 3 is reserved
                io_apic_address: u32::read_at(entry_data, 4).unwrap_or(0),
                gsi_base: u32::read_at(entry_data, 8).unwrap_or(0),
            }),

            // Type 2: Interrupt Source Override — 10 bytes total.
            2 if length >= 10 => MadtEntry::InterruptSourceOverride(InterruptSourceOverride {
                bus: entry_data[2],
                source: entry_data[3],
                gsi: u32::read_at(entry_data, 4).unwrap_or(0),
                flags: u16::read_at(entry_data, 8).unwrap_or(0),
            }),

            // Type 4: NMI Source — 8 bytes total.
            4 if length >= 8 => MadtEntry::NmiSource(NmiSource {
                flags: u16::read_at(entry_data, 2).unwrap_or(0),
                gsi: u32::read_at(entry_data, 4).unwrap_or(0),
            }),

            // Type 5: Local APIC NMI — 6 bytes total.
            5 if length >= 6 => MadtEntry::LocalApicNmi(LocalApicNmi {
                acpi_processor_id: entry_data[2],
                flags: u16::read_at(entry_data, 3).unwrap_or(0),
                lint: entry_data[5],
            }),

            // length is guaranteed <= 255 because it was read from a u8 field.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "MADT entry length fits in u8"
            )]
            _ => MadtEntry::Unknown {
                entry_type,
                length: length as u8,
            },
        };

        self.pos += length;

        Some(entry)
    }
}
