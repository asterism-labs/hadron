//! Multiple APIC Description Table (MADT) parsing.
//!
//! The MADT describes the interrupt controller topology of the system,
//! including local APICs, I/O APICs, interrupt source overrides, and NMI
//! sources.

use core::ptr;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// MADT table signature (`b"APIC"`).
pub const MADT_SIGNATURE: &[u8; 4] = b"APIC";

/// Raw MADT header fields that follow the SDT header.
#[derive(Debug, Clone, Copy)]
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
    /// Pointer to the start of the entry array.
    entries_ptr: *const u8,
    /// Total length of the entry data in bytes.
    entries_len: usize,
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
        let header_ptr = unsafe { handler.map_physical_region(phys, SdtHeader::SIZE) };
        // SAFETY: header_ptr is valid for SdtHeader::SIZE bytes.
        let header = unsafe { SdtHeader::read_from(header_ptr) };

        if &header.signature() != MADT_SIGNATURE {
            return Err(AcpiError::InvalidSignature);
        }

        let total_len = header.length() as usize;

        // Map the entire table.
        // SAFETY: phys is valid, total_len comes from the header.
        let table_ptr = unsafe { handler.map_physical_region(phys, total_len) };

        // Validate the checksum over the entire table.
        // SAFETY: table_ptr is valid for total_len bytes.
        if !unsafe { crate::sdt::validate_checksum(table_ptr, total_len) } {
            return Err(AcpiError::InvalidChecksum);
        }

        // Read the fixed MADT fields after the SDT header.
        // SAFETY: table is at least SdtHeader::SIZE + FIELDS_SIZE bytes.
        let fields: MadtHeaderFields =
            unsafe { ptr::read_unaligned(table_ptr.add(SdtHeader::SIZE).cast()) };

        let entries_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let entries_len = total_len.saturating_sub(entries_offset);
        // SAFETY: entries_offset < total_len as guaranteed by the table.
        let entries_ptr = unsafe { table_ptr.add(entries_offset) };

        Ok(Self {
            local_apic_address: fields.local_apic_address,
            flags: fields.flags,
            entries_ptr,
            entries_len,
        })
    }

    /// Returns an iterator over the MADT interrupt controller entries.
    #[must_use]
    pub fn entries(&self) -> MadtEntryIter {
        MadtEntryIter {
            ptr: self.entries_ptr,
            remaining: self.entries_len,
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
    /// Pointer to the current entry.
    ptr: *const u8,
    /// Remaining bytes in the entry region.
    remaining: usize,
}

impl Iterator for MadtEntryIter {
    type Item = MadtEntry;

    fn next(&mut self) -> Option<Self::Item> {
        // Each entry has at least a 2-byte header: type (u8) + length (u8).
        if self.remaining < 2 {
            return None;
        }

        // SAFETY: we have verified at least 2 bytes remain.
        let entry_type = unsafe { self.ptr.read() };
        let length = unsafe { self.ptr.add(1).read() } as usize;

        if length < 2 || length > self.remaining {
            return None;
        }

        let entry = match entry_type {
            // Type 0: Local APIC — 8 bytes total.
            0 if length >= 8 => {
                // SAFETY: we have verified the length.
                unsafe {
                    MadtEntry::LocalApic(LocalApic {
                        acpi_processor_id: self.ptr.add(2).read(),
                        apic_id: self.ptr.add(3).read(),
                        flags: ptr::read_unaligned(self.ptr.add(4).cast::<u32>()),
                    })
                }
            }

            // Type 1: I/O APIC — 12 bytes total.
            1 if length >= 12 => {
                // SAFETY: we have verified the length.
                unsafe {
                    MadtEntry::IoApic(IoApic {
                        io_apic_id: self.ptr.add(2).read(),
                        // byte 3 is reserved
                        io_apic_address: ptr::read_unaligned(self.ptr.add(4).cast::<u32>()),
                        gsi_base: ptr::read_unaligned(self.ptr.add(8).cast::<u32>()),
                    })
                }
            }

            // Type 2: Interrupt Source Override — 10 bytes total.
            2 if length >= 10 => {
                // SAFETY: we have verified the length.
                unsafe {
                    MadtEntry::InterruptSourceOverride(InterruptSourceOverride {
                        bus: self.ptr.add(2).read(),
                        source: self.ptr.add(3).read(),
                        gsi: ptr::read_unaligned(self.ptr.add(4).cast::<u32>()),
                        flags: ptr::read_unaligned(self.ptr.add(8).cast::<u16>()),
                    })
                }
            }

            // Type 4: NMI Source — 8 bytes total.
            4 if length >= 8 => {
                // SAFETY: we have verified the length.
                unsafe {
                    MadtEntry::NmiSource(NmiSource {
                        flags: ptr::read_unaligned(self.ptr.add(2).cast::<u16>()),
                        gsi: ptr::read_unaligned(self.ptr.add(4).cast::<u32>()),
                    })
                }
            }

            // Type 5: Local APIC NMI — 6 bytes total.
            5 if length >= 6 => {
                // SAFETY: we have verified the length.
                unsafe {
                    MadtEntry::LocalApicNmi(LocalApicNmi {
                        acpi_processor_id: self.ptr.add(2).read(),
                        flags: ptr::read_unaligned(self.ptr.add(3).cast::<u16>()),
                        lint: self.ptr.add(5).read(),
                    })
                }
            }

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

        // Advance past this entry.
        // SAFETY: length <= self.remaining, so the new pointer is within bounds.
        self.ptr = unsafe { self.ptr.add(length) };
        self.remaining -= length;

        Some(entry)
    }
}
