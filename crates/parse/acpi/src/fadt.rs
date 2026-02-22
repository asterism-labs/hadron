//! Fixed ACPI Description Table (FADT) parsing.
//!
//! The FADT contains fixed hardware configuration data needed by the ACPI
//! driver. We parse the PM timer I/O port, the century CMOS register index,
//! boot architecture flags, general ACPI feature flags, and the physical
//! addresses of the DSDT and FACS tables.

use hadron_binparse::FromBytes;

use crate::{AcpiError, AcpiHandler};

/// FADT table signature.
pub const FADT_SIGNATURE: &[u8; 4] = b"FACP";

/// Parsed FADT — only the fields we currently need.
#[derive(Debug, Clone, Copy)]
pub struct Fadt {
    /// Physical address of the FACS (Firmware ACPI Control Structure).
    ///
    /// 32-bit field at offset 36. Zero if not present.
    pub firmware_ctrl: u32,
    /// Physical address of the DSDT (Differentiated System Description Table).
    ///
    /// 32-bit field at offset 40. Zero if not present.
    pub dsdt: u32,
    /// I/O port address of the PM Timer (Power Management Timer).
    ///
    /// If zero, the PM timer is not available via a fixed I/O port.
    pub pm_timer_block: u32,
    /// CMOS RAM index of the century BCD value (RTC).
    ///
    /// If zero, the century register is not supported.
    pub century: u8,
    /// IA-PC boot architecture flags.
    ///
    /// Bit 0: legacy 8259 IRQ routing, bit 1: 8042 controller, etc.
    pub boot_architecture_flags: u16,
    /// Fixed feature flags.
    ///
    /// See the ACPI specification for individual bit meanings.
    pub flags: u32,
    /// 64-bit physical address of the FACS (ACPI 2.0+).
    ///
    /// Zero if not present or table predates ACPI 2.0.
    pub x_firmware_ctrl: u64,
    /// 64-bit physical address of the DSDT (ACPI 2.0+).
    ///
    /// Zero if not present or table predates ACPI 2.0.
    pub x_dsdt: u64,
}

impl Fadt {
    /// Byte offset of `firmware_ctrl` within the FADT.
    const FIRMWARE_CTRL_OFFSET: usize = 36;
    /// Byte offset of `dsdt` within the FADT.
    const DSDT_OFFSET: usize = 40;
    /// Byte offset of `pm_tmr_blk` within the FADT (from the start of the table).
    const PM_TMR_BLK_OFFSET: usize = 76;
    /// Byte offset of `century` within the FADT.
    const CENTURY_OFFSET: usize = 108;
    /// Byte offset of `boot_architecture_flags` (`IAPC_BOOT_ARCH`) within the FADT.
    const BOOT_ARCH_OFFSET: usize = 109;
    /// Byte offset of `flags` within the FADT.
    const FLAGS_OFFSET: usize = 112;
    /// Byte offset of `x_firmware_ctrl` within the FADT (ACPI 2.0+).
    const X_FIRMWARE_CTRL_OFFSET: usize = 132;
    /// Byte offset of `x_dsdt` within the FADT (ACPI 2.0+).
    const X_DSDT_OFFSET: usize = 140;

    /// Minimum FADT length required to read all the fields we need.
    ///
    /// We need up to offset 140 + 8 bytes = 148 bytes for x_dsdt.
    const MIN_LENGTH: usize = 148;

    /// Parse a FADT from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `FACP`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, FADT_SIGNATURE)?;

        // Older FADT revisions may be shorter; provide zero defaults for
        // missing fields rather than failing outright.
        if table.data.len() < Self::MIN_LENGTH {
            return Ok(Self::parse_partial(table.data));
        }

        Ok(Self::read_fields(table.data))
    }

    /// Read all needed fields from a fully-sized FADT byte slice.
    fn read_fields(data: &[u8]) -> Self {
        Self {
            firmware_ctrl: u32::read_at(data, Self::FIRMWARE_CTRL_OFFSET).unwrap_or(0),
            dsdt: u32::read_at(data, Self::DSDT_OFFSET).unwrap_or(0),
            pm_timer_block: u32::read_at(data, Self::PM_TMR_BLK_OFFSET).unwrap_or(0),
            century: u8::read_at(data, Self::CENTURY_OFFSET).unwrap_or(0),
            boot_architecture_flags: u16::read_at(data, Self::BOOT_ARCH_OFFSET).unwrap_or(0),
            flags: u32::read_at(data, Self::FLAGS_OFFSET).unwrap_or(0),
            x_firmware_ctrl: u64::read_at(data, Self::X_FIRMWARE_CTRL_OFFSET).unwrap_or(0),
            x_dsdt: u64::read_at(data, Self::X_DSDT_OFFSET).unwrap_or(0),
        }
    }

    /// Parse a shorter-than-expected FADT, filling in zero for missing fields.
    fn parse_partial(data: &[u8]) -> Self {
        Self {
            firmware_ctrl: u32::read_at(data, Self::FIRMWARE_CTRL_OFFSET).unwrap_or(0),
            dsdt: u32::read_at(data, Self::DSDT_OFFSET).unwrap_or(0),
            pm_timer_block: u32::read_at(data, Self::PM_TMR_BLK_OFFSET).unwrap_or(0),
            century: u8::read_at(data, Self::CENTURY_OFFSET).unwrap_or(0),
            boot_architecture_flags: u16::read_at(data, Self::BOOT_ARCH_OFFSET).unwrap_or(0),
            flags: u32::read_at(data, Self::FLAGS_OFFSET).unwrap_or(0),
            x_firmware_ctrl: u64::read_at(data, Self::X_FIRMWARE_CTRL_OFFSET).unwrap_or(0),
            x_dsdt: u64::read_at(data, Self::X_DSDT_OFFSET).unwrap_or(0),
        }
    }

    /// Returns the physical address of the DSDT.
    ///
    /// Prefers the 64-bit `x_dsdt` field (ACPI 2.0+) if non-zero, otherwise
    /// falls back to the 32-bit `dsdt` field. Returns `None` if both are zero.
    #[must_use]
    pub fn dsdt_address(&self) -> Option<u64> {
        if self.x_dsdt != 0 {
            Some(self.x_dsdt)
        } else if self.dsdt != 0 {
            Some(u64::from(self.dsdt))
        } else {
            None
        }
    }

    /// Returns the physical address of the FACS.
    ///
    /// Prefers the 64-bit `x_firmware_ctrl` field (ACPI 2.0+) if non-zero,
    /// otherwise falls back to the 32-bit `firmware_ctrl` field. Returns
    /// `None` if both are zero.
    #[must_use]
    pub fn facs_address(&self) -> Option<u64> {
        if self.x_firmware_ctrl != 0 {
            Some(self.x_firmware_ctrl)
        } else if self.firmware_ctrl != 0 {
            Some(u64::from(self.firmware_ctrl))
        } else {
            None
        }
    }
}
