//! Intel e1000e (82574L) register definitions and descriptor types.
//!
//! Defines the memory-mapped register layout using [`register_block!`], typed
//! bitflags for control/status registers, and legacy 16-byte TX/RX descriptor
//! structures.

use bitflags::bitflags;
use hadron_kernel::addr::VirtAddr;
use hadron_mmio::register_block;

// ---------------------------------------------------------------------------
// Register block
// ---------------------------------------------------------------------------

register_block! {
    /// Intel e1000e MMIO register block.
    pub E1000eRegs {
        /// Device Control Register.
        [0x0000; u32; rw] ctrl => Ctrl,
        /// Device Status Register.
        [0x0008; u32; ro] status => Status,
        /// EEPROM Read Register.
        [0x0014; u32; rw] eerd,
        /// Interrupt Cause Read (read-clears pending causes).
        [0x00C0; u32; rw] icr,
        /// Interrupt Mask Set/Read.
        [0x00D0; u32; rw] ims,
        /// Interrupt Mask Clear (write-only).
        [0x00D8; u32; wo] imc,
        /// Receive Control Register.
        [0x0100; u32; rw] rctl => Rctl,
        /// Transmit Control Register.
        [0x0400; u32; rw] tctl => Tctl,
        /// Transmit Inter-Packet Gap.
        [0x0410; u32; rw] tipg,
        /// RX Descriptor Base Address Low.
        [0x2800; u32; rw] rdbal,
        /// RX Descriptor Base Address High.
        [0x2804; u32; rw] rdbah,
        /// RX Descriptor Ring Length (bytes).
        [0x2808; u32; rw] rdlen,
        /// RX Descriptor Head.
        [0x2810; u32; rw] rdh,
        /// RX Descriptor Tail.
        [0x2818; u32; rw] rdt,
        /// TX Descriptor Base Address Low.
        [0x3800; u32; rw] tdbal,
        /// TX Descriptor Base Address High.
        [0x3804; u32; rw] tdbah,
        /// TX Descriptor Ring Length (bytes).
        [0x3808; u32; rw] tdlen,
        /// TX Descriptor Head.
        [0x3810; u32; rw] tdh,
        /// TX Descriptor Tail.
        [0x3818; u32; rw] tdt,
        /// Receive Address Low (first entry).
        [0x5400; u32; ro] ral0,
        /// Receive Address High (first entry).
        [0x5404; u32; rw] rah0,
    }
}

// ---------------------------------------------------------------------------
// Control register bitflags
// ---------------------------------------------------------------------------

bitflags! {
    /// Device Control (CTRL) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct Ctrl: u32 {
        /// Set Link Up — forces the PHY link to "up".
        const SLU = 1 << 6;
        /// Device Reset — self-clears after ~1 µs.
        const RST = 1 << 26;
    }
}

bitflags! {
    /// Device Status register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct Status: u32 {
        /// Link Up indication from the PHY.
        const LU = 1 << 1;
    }
}

bitflags! {
    /// Receive Control (RCTL) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct Rctl: u32 {
        /// Receiver Enable.
        const EN = 1 << 1;
        /// Broadcast Accept Mode.
        const BAM = 1 << 15;
        /// Strip Ethernet CRC from received frames.
        const SECRC = 1 << 26;
    }
}

bitflags! {
    /// Transmit Control (TCTL) register flags.
    #[derive(Debug, Clone, Copy)]
    pub struct Tctl: u32 {
        /// Transmitter Enable.
        const EN = 1 << 1;
        /// Pad Short Packets to 64 bytes.
        const PSP = 1 << 3;
    }
}

// ---------------------------------------------------------------------------
// Interrupt cause bits (shared by ICR / IMS / IMC)
// ---------------------------------------------------------------------------

/// TX Descriptor Written Back.
pub const ICR_TXDW: u32 = 1 << 0;
/// Link Status Change.
pub const ICR_LSC: u32 = 1 << 2;
/// RX Descriptor Minimum Threshold reached.
pub const ICR_RXDMT: u32 = 1 << 4;
/// RX Timer expired (packet received).
pub const ICR_RXT0: u32 = 1 << 7;

// ---------------------------------------------------------------------------
// Descriptor structures (legacy 16-byte format)
// ---------------------------------------------------------------------------

/// Legacy RX descriptor (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RxDesc {
    /// Physical address of the receive buffer.
    pub addr: u64,
    /// Length of the received packet (filled by hardware).
    pub length: u16,
    /// Packet checksum (filled by hardware).
    pub checksum: u16,
    /// Status bits (DD, EOP, etc.).
    pub status: u8,
    /// Error bits.
    pub errors: u8,
    /// Special / VLAN tag.
    pub special: u16,
}

/// Legacy TX descriptor (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TxDesc {
    /// Physical address of the transmit buffer.
    pub addr: u64,
    /// Length of the packet data.
    pub length: u16,
    /// Checksum offset.
    pub cso: u8,
    /// Command bits (EOP, IFCS, RS, etc.).
    pub cmd: u8,
    /// Status bits (DD when writeback completes).
    pub status: u8,
    /// Checksum start.
    pub css: u8,
    /// Special / VLAN tag.
    pub special: u16,
}

// ---------------------------------------------------------------------------
// RX descriptor status bits
// ---------------------------------------------------------------------------

/// Descriptor Done — hardware has processed this descriptor.
pub const RXD_STAT_DD: u8 = 0x01;
/// End of Packet — this descriptor contains the last fragment.
pub const RXD_STAT_EOP: u8 = 0x02;

// ---------------------------------------------------------------------------
// TX descriptor command bits
// ---------------------------------------------------------------------------

/// End of Packet — marks the last descriptor in a frame.
pub const TXD_CMD_EOP: u8 = 0x01;
/// Insert FCS/CRC — hardware appends the CRC.
pub const TXD_CMD_IFCS: u8 = 0x02;
/// Report Status — hardware sets DD in status after send.
pub const TXD_CMD_RS: u8 = 0x08;

// ---------------------------------------------------------------------------
// TX descriptor status bits
// ---------------------------------------------------------------------------

/// Descriptor Done — hardware has transmitted this descriptor.
pub const TXD_STAT_DD: u8 = 0x01;

// ---------------------------------------------------------------------------
// Multicast Table Array
// ---------------------------------------------------------------------------

/// Base offset of the MTA (128 × 32-bit entries).
pub const MTA_BASE: u64 = 0x5200;
/// Number of 32-bit entries in the MTA.
pub const MTA_COUNT: usize = 128;
