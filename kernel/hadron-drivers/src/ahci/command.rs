//! AHCI command structures: FIS, Command Header, and PRDT entries.
//!
//! All structures use `#[repr(C, packed)]` to match the hardware-mandated
//! layout exactly.

/// FIS Register — Host to Device (20 bytes).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct FisRegH2d {
    /// FIS type (0x27 for Register H2D).
    pub fis_type: u8,
    /// PM port and C bit (bit 7 = 1 for command, 0 for control).
    pub pm_and_c: u8,
    /// ATA command register.
    pub command: u8,
    /// Features register (low byte).
    pub features_lo: u8,

    /// LBA low byte (bits 7:0).
    pub lba0: u8,
    /// LBA mid byte (bits 15:8).
    pub lba1: u8,
    /// LBA high byte (bits 23:16).
    pub lba2: u8,
    /// Device register.
    pub device: u8,

    /// LBA register (bits 31:24).
    pub lba3: u8,
    /// LBA register (bits 39:32).
    pub lba4: u8,
    /// LBA register (bits 47:40).
    pub lba5: u8,
    /// Features register (high byte).
    pub features_hi: u8,

    /// Sector count (low byte).
    pub count_lo: u8,
    /// Sector count (high byte).
    pub count_hi: u8,
    /// Isochronous command completion.
    pub icc: u8,
    /// Control register.
    pub control: u8,

    /// Reserved.
    pub _reserved: [u8; 4],
}

impl FisRegH2d {
    /// Creates a zeroed FIS Register H2D.
    #[must_use]
    pub const fn zeroed() -> Self {
        Self {
            fis_type: 0,
            pm_and_c: 0,
            command: 0,
            features_lo: 0,
            lba0: 0,
            lba1: 0,
            lba2: 0,
            device: 0,
            lba3: 0,
            lba4: 0,
            lba5: 0,
            features_hi: 0,
            count_lo: 0,
            count_hi: 0,
            icc: 0,
            control: 0,
            _reserved: [0; 4],
        }
    }
}

/// AHCI Command Header (32 bytes), one per command slot.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct CommandHeader {
    /// DW0: flags (CFL in bits 4:0, A=bit5, W=bit6, P=bit7, R=bit8, B=bit9, C=bit10, PMP=bits 15:12).
    pub flags: u16,
    /// Physical Region Descriptor Table Length (entries).
    pub prdtl: u16,
    /// Physical Region Descriptor Byte Count (returned by HBA).
    pub prdbc: u32,
    /// Command Table Base Address (low 32 bits, 128-byte aligned).
    pub ctba: u32,
    /// Command Table Base Address (high 32 bits).
    pub ctbau: u32,
    /// Reserved.
    pub _reserved: [u32; 4],
}

/// Physical Region Descriptor Table entry (16 bytes).
#[repr(C, packed)]
#[derive(Debug, Clone, Copy)]
pub struct PrdtEntry {
    /// Data Base Address (low 32 bits, word-aligned).
    pub dba: u32,
    /// Data Base Address (high 32 bits).
    pub dbau: u32,
    /// Reserved.
    pub _reserved: u32,
    /// Data Byte Count (bit 31 = Interrupt on Completion). Max 4 MiB per entry.
    pub dbc: u32,
}

/// Offset of the Command FIS within a Command Table.
pub const CMD_FIS_OFFSET: usize = 0x00;
/// Offset of the PRDT within a Command Table.
pub const PRDT_OFFSET: usize = 0x80;
/// Maximum PRDT entries per command table.
pub const MAX_PRDT_ENTRIES: usize = 248;
/// Command FIS length in DWORDs for Register H2D (20 bytes / 4 = 5).
pub const CMD_FIS_LEN_DWORDS: u16 = 5;
