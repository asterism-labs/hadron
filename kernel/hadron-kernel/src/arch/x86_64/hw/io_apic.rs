//! I/O APIC driver.
//!
//! Controls external interrupt routing from hardware devices to Local APICs.

use crate::addr::VirtAddr;

const IOREGSEL: u32 = 0x00;
const IOWIN: u32 = 0x10;

const REG_ID: u32 = 0x00;
const REG_VER: u32 = 0x01;

/// Delivery modes for I/O APIC redirection entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DeliveryMode {
    /// Fixed delivery to specified processor(s).
    Fixed = 0b000,
    /// Lowest priority delivery.
    LowPriority = 0b001,
    /// System Management Interrupt.
    Smi = 0b010,
    /// Non-Maskable Interrupt.
    Nmi = 0b100,
    /// INIT signal.
    Init = 0b101,
    /// External interrupt.
    ExtInt = 0b111,
}

/// Destination mode for I/O APIC redirection entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationMode {
    /// Physical APIC ID.
    Physical,
    /// Logical APIC ID.
    Logical,
}

/// Pin polarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// Active high.
    ActiveHigh,
    /// Active low.
    ActiveLow,
}

/// Trigger mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerMode {
    /// Edge triggered.
    Edge,
    /// Level triggered.
    Level,
}

/// An I/O APIC redirection table entry.
#[derive(Debug, Clone, Copy)]
pub struct RedirectionEntry {
    /// Interrupt vector (32-255).
    pub vector: u8,
    /// Delivery mode.
    pub delivery_mode: DeliveryMode,
    /// Destination mode.
    pub destination_mode: DestinationMode,
    /// Pin polarity.
    pub polarity: Polarity,
    /// Trigger mode.
    pub trigger_mode: TriggerMode,
    /// Whether the interrupt is masked.
    pub masked: bool,
    /// Destination APIC ID.
    pub destination: u8,
}

impl RedirectionEntry {
    /// Encodes this entry as a 64-bit register value.
    fn encode(&self) -> u64 {
        let mut val: u64 = u64::from(self.vector);
        val |= u64::from(self.delivery_mode as u8) << 8;
        if self.destination_mode == DestinationMode::Logical {
            val |= 1 << 11;
        }
        if self.polarity == Polarity::ActiveLow {
            val |= 1 << 13;
        }
        if self.trigger_mode == TriggerMode::Level {
            val |= 1 << 15;
        }
        if self.masked {
            val |= 1 << 16;
        }
        val |= u64::from(self.destination) << 56;
        val
    }
}

/// I/O APIC driver using indirect MMIO register access.
pub struct IoApic {
    base: VirtAddr,
    gsi_base: u32,
}

impl IoApic {
    /// Creates a new I/O APIC driver.
    ///
    /// # Safety
    ///
    /// `virt_base` must be a valid mapping of the I/O APIC MMIO region.
    /// `gsi_base` is the Global System Interrupt base for this I/O APIC.
    pub unsafe fn new(virt_base: VirtAddr, gsi_base: u32) -> Self {
        Self {
            base: virt_base,
            gsi_base,
        }
    }

    /// Returns the I/O APIC ID.
    pub fn id(&self) -> u8 {
        ((self.read_reg(REG_ID) >> 24) & 0x0F) as u8
    }

    /// Returns the maximum redirection entry index.
    pub fn max_redirection_entry(&self) -> u8 {
        ((self.read_reg(REG_VER) >> 16) & 0xFF) as u8
    }

    /// Returns the GSI base for this I/O APIC.
    pub fn gsi_base(&self) -> u32 {
        self.gsi_base
    }

    /// Sets a redirection table entry.
    pub fn set_entry(&self, index: u8, entry: RedirectionEntry) {
        let encoded = entry.encode();
        let reg_low = 0x10 + 2 * u32::from(index);
        let reg_high = reg_low + 1;
        self.write_reg(reg_high, (encoded >> 32) as u32);
        self.write_reg(reg_low, encoded as u32);
    }

    /// Masks (disables) a redirection table entry.
    pub fn mask(&self, index: u8) {
        let reg_low = 0x10 + 2 * u32::from(index);
        let val = self.read_reg(reg_low);
        self.write_reg(reg_low, val | (1 << 16));
    }

    /// Unmasks (enables) a redirection table entry.
    pub fn unmask(&self, index: u8) {
        let reg_low = 0x10 + 2 * u32::from(index);
        let val = self.read_reg(reg_low);
        self.write_reg(reg_low, val & !(1 << 16));
    }

    /// Reads the raw low and high dwords of a redirection table entry.
    ///
    /// Useful for diagnostics: low dword contains vector, delivery mode, mask
    /// bit; high dword contains the destination APIC ID.
    pub fn read_entry_raw(&self, index: u8) -> (u32, u32) {
        let reg_low = 0x10 + 2 * u32::from(index);
        let reg_high = reg_low + 1;
        let low = self.read_reg(reg_low);
        let high = self.read_reg(reg_high);
        (low, high)
    }

    #[inline]
    fn read_reg(&self, reg: u32) -> u32 {
        // SAFETY: The caller of `IoApic::new` guarantees that `self.base` points to
        // a valid I/O APIC MMIO region. IOREGSEL and IOWIN are within the mapped page.
        unsafe {
            let sel = (self.base.as_u64() + u64::from(IOREGSEL)) as *mut u32;
            let win = (self.base.as_u64() + u64::from(IOWIN)) as *const u32;
            core::ptr::write_volatile(sel, reg);
            core::ptr::read_volatile(win)
        }
    }

    #[inline]
    fn write_reg(&self, reg: u32, value: u32) {
        // SAFETY: The caller of `IoApic::new` guarantees that `self.base` points to
        // a valid I/O APIC MMIO region. IOREGSEL and IOWIN are within the mapped page.
        unsafe {
            let sel = (self.base.as_u64() + u64::from(IOREGSEL)) as *mut u32;
            let win = (self.base.as_u64() + u64::from(IOWIN)) as *mut u32;
            core::ptr::write_volatile(sel, reg);
            core::ptr::write_volatile(win, value);
        }
    }
}
