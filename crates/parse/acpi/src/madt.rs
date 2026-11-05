//! Multiple APIC Description Table (MADT) parsing.
//!
//! The MADT describes the interrupt controller topology of the system,
//! including local APICs, I/O APICs, interrupt source overrides, and NMI
//! sources.
//!
//! # References
//!
//! - ACPI Specification 6.5, §5.2.12: Multiple APIC Description Table (MADT)
//!   <https://uefi.org/specs/ACPI/6.5/>

use crate::read::{split_u8_entry, u8_at, u16_at, u32_at, u64_at};
use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// MADT table signature (`b"APIC"`).
pub const MADT_SIGNATURE: &[u8; 4] = b"APIC";

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

        let local_apic_address =
            u32_at(table.data, SdtHeader::SIZE).ok_or(AcpiError::TruncatedData)?;
        let flags = u32_at(table.data, SdtHeader::SIZE + 4).ok_or(AcpiError::TruncatedData)?;

        let entries_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let entries_data = table.data.get(entries_offset..).unwrap_or(&[]);

        Ok(Self {
            local_apic_address,
            flags,
            entries_data,
        })
    }

    /// Returns an iterator over the MADT interrupt controller entries.
    #[must_use]
    pub fn entries(&self) -> MadtEntryIter<'_> {
        MadtEntryIter {
            data: self.entries_data,
        }
    }
}

/// A single MADT interrupt controller structure entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MadtEntry {
    /// Type 0: Processor Local APIC.
    LocalApic {
        /// ACPI processor UID.
        acpi_processor_id: u8,
        /// The processor's local APIC ID.
        apic_id: u8,
        /// Flags (bit 0: enabled, bit 1: online capable).
        flags: u32,
    },

    /// Type 1: I/O APIC.
    IoApic {
        /// The I/O APIC ID.
        io_apic_id: u8,
        /// Physical address of the I/O APIC registers.
        io_apic_address: u32,
        /// Global System Interrupt base for this I/O APIC.
        gsi_base: u32,
    },

    /// Type 2: Interrupt Source Override.
    InterruptSourceOverride {
        /// Constant: 0 (ISA bus).
        bus: u8,
        /// ISA source IRQ number.
        source: u8,
        /// Global System Interrupt number this source maps to.
        gsi: u32,
        /// MPS INTI flags.
        flags: u16,
    },

    /// Type 3: Non-Maskable Interrupt Source.
    NmiSource {
        /// MPS INTI flags.
        flags: u16,
        /// Global System Interrupt number of the NMI source.
        gsi: u32,
    },

    /// Type 4: Local APIC NMI.
    LocalApicNmi {
        /// ACPI processor UID (0xFF means all processors).
        acpi_processor_id: u8,
        /// MPS INTI flags.
        flags: u16,
        /// Local APIC LINT pin (0 or 1).
        lint: u8,
    },

    /// Type 5: Local APIC Address Override. When present, every local APIC
    /// is at this 64-bit address instead of [`Madt::local_apic_address`].
    LocalApicAddressOverride {
        /// Physical address of the local APIC.
        address: u64,
    },

    /// An entry type that we do not parse, or a known type that is shorter
    /// than its fixed fields.
    Unknown {
        /// The entry type byte.
        entry_type: u8,
        /// The entry length including the 2-byte header.
        length: u8,
    },
}

impl MadtEntry {
    /// Decodes one entry, header included.
    ///
    /// Returns `None` when a field of a known type lies past the end of
    /// `entry`; the caller reports that entry as [`MadtEntry::Unknown`].
    fn decode(entry_type: u8, entry: &[u8]) -> Option<Self> {
        Some(match entry_type {
            0 => Self::LocalApic {
                acpi_processor_id: u8_at(entry, 2)?,
                apic_id: u8_at(entry, 3)?,
                flags: u32_at(entry, 4)?,
            },
            1 => Self::IoApic {
                io_apic_id: u8_at(entry, 2)?,
                io_apic_address: u32_at(entry, 4)?,
                gsi_base: u32_at(entry, 8)?,
            },
            2 => Self::InterruptSourceOverride {
                bus: u8_at(entry, 2)?,
                source: u8_at(entry, 3)?,
                gsi: u32_at(entry, 4)?,
                flags: u16_at(entry, 8)?,
            },
            3 => Self::NmiSource {
                flags: u16_at(entry, 2)?,
                gsi: u32_at(entry, 4)?,
            },
            4 => Self::LocalApicNmi {
                acpi_processor_id: u8_at(entry, 2)?,
                flags: u16_at(entry, 3)?,
                lint: u8_at(entry, 5)?,
            },
            5 => Self::LocalApicAddressOverride {
                address: u64_at(entry, 4)?,
            },
            _ => return None,
        })
    }
}

/// Iterator over MADT interrupt controller entries.
///
/// Iteration ends at the first entry whose length is below 2 or runs past
/// the end of the table.
pub struct MadtEntryIter<'a> {
    data: &'a [u8],
}

impl Iterator for MadtEntryIter<'_> {
    type Item = MadtEntry;

    fn next(&mut self) -> Option<Self::Item> {
        let entry = split_u8_entry(&mut self.data)?;
        let (entry_type, length) = (entry[0], entry[1]);
        Some(
            MadtEntry::decode(entry_type, entry)
                .unwrap_or(MadtEntry::Unknown { entry_type, length }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(data: &[u8]) -> MadtEntryIter<'_> {
        MadtEntryIter { data }
    }

    #[test]
    fn decodes_known_entries() {
        #[rustfmt::skip]
        let data = [
            // Local APIC: uid 1, apic id 2, flags 1.
            0, 8, 1, 2, 1, 0, 0, 0,
            // I/O APIC: id 3, address 0xFEC0_0000, GSI base 0.
            1, 12, 3, 0, 0x00, 0x00, 0xC0, 0xFE, 0, 0, 0, 0,
            // Interrupt source override: IRQ 0 -> GSI 2, flags 0x000F.
            2, 10, 0, 0, 2, 0, 0, 0, 0x0F, 0x00,
            // NMI source: flags 0x000D, GSI 7.
            3, 8, 0x0D, 0x00, 7, 0, 0, 0,
            // Local APIC NMI at an unaligned u16.
            4, 6, 0xFF, 0x05, 0x00, 1,
            // Local APIC address override: 0x1_FEE0_0000.
            5, 12, 0, 0, 0x00, 0x00, 0xE0, 0xFE, 1, 0, 0, 0,
        ];
        let mut iter = entries(&data);
        assert_eq!(
            iter.next(),
            Some(MadtEntry::LocalApic {
                acpi_processor_id: 1,
                apic_id: 2,
                flags: 1
            })
        );
        assert_eq!(
            iter.next(),
            Some(MadtEntry::IoApic {
                io_apic_id: 3,
                io_apic_address: 0xFEC0_0000,
                gsi_base: 0
            })
        );
        assert_eq!(
            iter.next(),
            Some(MadtEntry::InterruptSourceOverride {
                bus: 0,
                source: 0,
                gsi: 2,
                flags: 0x0F
            })
        );
        assert_eq!(
            iter.next(),
            Some(MadtEntry::NmiSource {
                flags: 0x0D,
                gsi: 7
            })
        );
        assert_eq!(
            iter.next(),
            Some(MadtEntry::LocalApicNmi {
                acpi_processor_id: 0xFF,
                flags: 5,
                lint: 1
            })
        );
        assert_eq!(
            iter.next(),
            Some(MadtEntry::LocalApicAddressOverride {
                address: 0x1_FEE0_0000
            })
        );
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn short_known_entry_is_unknown() {
        // An I/O APIC entry that claims 4 bytes instead of 12.
        let data = [1, 4, 3, 0];
        let mut iter = entries(&data);
        assert_eq!(
            iter.next(),
            Some(MadtEntry::Unknown {
                entry_type: 1,
                length: 4
            })
        );
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn malformed_length_ends_iteration() {
        // Second entry has length 0; third would run past the end.
        let data = [9, 2, 0, 0, 0, 8, 1];
        let mut iter = entries(&data);
        assert_eq!(
            iter.next(),
            Some(MadtEntry::Unknown {
                entry_type: 9,
                length: 2
            })
        );
        assert_eq!(iter.next(), None);
        assert_eq!(iter.next(), None);
    }
}
