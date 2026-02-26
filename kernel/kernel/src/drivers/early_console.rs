//! Minimal serial port driver for early boot logging and panic output.
//!
//! Provides raw port I/O to a 16550-compatible UART without depending on
//! the full UART driver in `hadron-drivers`. The boot stub initializes
//! the hardware before calling into the kernel; this module only needs
//! to read/write bytes.

#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::Port;

/// Standard COM1 base I/O port address.
pub const COM1: u16 = 0x3F8;

/// Line Status Register offset from UART base.
const LSR_OFFSET: u16 = 5;

/// LSR bit: Transmitter Holding Register Empty.
const LSR_THRE: u8 = 1 << 5;

/// LSR bit: Data Ready (receive buffer has data).
const LSR_DR: u8 = 1 << 0;

/// A minimal serial port handle for early boot I/O.
///
/// Unlike the full `Uart16550` driver, this type performs no initialization
/// and carries no state beyond the base I/O port address. It is safe to
/// construct from any context (including panics with the logger lock held).
#[derive(Debug, Clone, Copy)]
pub struct EarlySerial {
    base: u16,
}

impl EarlySerial {
    /// Creates a new early serial handle for the given base port.
    #[must_use]
    pub const fn new(base: u16) -> Self {
        Self { base }
    }

    /// Writes a single byte, spinning until the transmit buffer is ready.
    #[cfg(target_arch = "x86_64")]
    pub fn write_byte(&self, byte: u8) {
        // Wait for THR empty.
        // SAFETY: Reading LSR is a side-effect-free status register read.
        while unsafe { Port::<u8>::new(self.base + LSR_OFFSET).read() } & LSR_THRE == 0 {
            core::hint::spin_loop();
        }
        // SAFETY: Writing to THR sends one byte over the serial line.
        unsafe { Port::<u8>::new(self.base).write(byte) };
    }

    /// Reads a single byte if data is available (non-blocking).
    #[cfg(target_arch = "x86_64")]
    #[must_use]
    pub fn try_read_byte(&self) -> Option<u8> {
        // SAFETY: Reading LSR is a side-effect-free status register read.
        let lsr = unsafe { Port::<u8>::new(self.base + LSR_OFFSET).read() };
        if lsr & LSR_DR != 0 {
            // SAFETY: Reading RBR retrieves one byte from the receive buffer.
            Some(unsafe { Port::<u8>::new(self.base).read() })
        } else {
            None
        }
    }
}
