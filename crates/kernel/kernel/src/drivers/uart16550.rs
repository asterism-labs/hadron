//! Polled driver for a 16550-compatible UART reached through x86 I/O ports.
//!
//! The driver only transmits. It never enables the UART's interrupts and
//! waits on the line status register before each byte.

use crate::arch::x86_64::port::{inb, outb};

/// The UART's input clock divided by 16: the baud rate at divisor 1.
const MAX_BAUD: u32 = 115_200;

// Register offsets from the base port.
/// Transmit holding register (write) and receive buffer (read). With
/// `LCR_DLAB` set, the divisor's low byte.
const DATA: u16 = 0;
/// Interrupt enable register. With `LCR_DLAB` set, the divisor's high byte.
const IER: u16 = 1;
/// FIFO control register (write only).
const FCR: u16 = 2;
/// Line control register.
const LCR: u16 = 3;
/// Modem control register.
const MCR: u16 = 4;
/// Line status register.
const LSR: u16 = 5;

/// `LCR`: eight data bits, no parity, one stop bit.
const LCR_8N1: u8 = 0b0000_0011;
/// `LCR`: divisor latch access. Turns `DATA` and `IER` into the divisor.
const LCR_DLAB: u8 = 1 << 7;
/// `FCR`: enable both FIFOs, clear them, and set the receive trigger to
/// 14 bytes.
const FCR_ENABLE_CLEAR_14: u8 = 0b1100_0111;
/// `MCR`: data terminal ready.
const MCR_DTR: u8 = 1 << 0;
/// `MCR`: request to send.
const MCR_RTS: u8 = 1 << 1;
/// `MCR`: auxiliary output 1.
const MCR_OUT1: u8 = 1 << 2;
/// `MCR`: auxiliary output 2, which gates the UART's interrupt line on PCs.
const MCR_OUT2: u8 = 1 << 3;
/// `MCR`: loopback. The transmitter feeds the receiver and nothing reaches
/// the line.
const MCR_LOOPBACK: u8 = 1 << 4;
/// `LSR`: a received byte is waiting in `DATA`.
const LSR_DATA_READY: u8 = 1 << 0;
/// `LSR`: the transmit holding register can take a byte.
const LSR_THR_EMPTY: u8 = 1 << 5;

/// Byte sent through loopback during [`Uart16550::init`].
const LOOPBACK_PROBE: u8 = 0xae;
/// How many times [`Uart16550::init`] reads `LSR` while waiting for the
/// probe byte. Each read is an I/O port access of roughly a microsecond.
const LOOPBACK_POLLS: u32 = 1000;

/// Why [`Uart16550::init`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InitError {
    /// The baud rate is zero, above 115200, or does not divide 115200.
    UnsupportedBaud(u32),
    /// The byte sent in loopback mode did not come back, so no working
    /// UART sits at this port.
    LoopbackFailed {
        /// The byte read back, if any arrived.
        received: Option<u8>,
    },
}

/// A 16550 UART at a fixed I/O port base.
#[derive(Debug)]
pub(crate) struct Uart16550 {
    base: u16,
}

impl Uart16550 {
    /// Names the UART whose eight registers start at port `base`.
    ///
    /// # Safety
    ///
    /// Ports `base..base + 8` must belong to a 16550 or to nothing, and no
    /// other code may use them.
    pub(crate) const unsafe fn new(base: u16) -> Self {
        Self { base }
    }

    /// Sets the UART to `baud`, 8N1, FIFOs on, interrupts off, and checks
    /// it with a loopback round trip.
    pub(crate) fn init(&self, baud: u32) -> Result<(), InitError> {
        if baud == 0 || baud > MAX_BAUD || MAX_BAUD % baud != 0 {
            return Err(InitError::UnsupportedBaud(baud));
        }
        let [divisor_low, divisor_high, ..] = (MAX_BAUD / baud).to_le_bytes();

        self.write(IER, 0);
        self.write(LCR, LCR_DLAB);
        self.write(DATA, divisor_low);
        self.write(IER, divisor_high);
        self.write(LCR, LCR_8N1);
        self.write(FCR, FCR_ENABLE_CLEAR_14);

        self.write(MCR, MCR_LOOPBACK | MCR_RTS | MCR_OUT1 | MCR_OUT2);
        self.write(DATA, LOOPBACK_PROBE);
        let received = (0..LOOPBACK_POLLS)
            .find(|_| self.read(LSR) & LSR_DATA_READY != 0)
            .map(|_| self.read(DATA));
        if received != Some(LOOPBACK_PROBE) {
            return Err(InitError::LoopbackFailed { received });
        }

        self.write(MCR, MCR_DTR | MCR_RTS | MCR_OUT2);
        Ok(())
    }

    /// Sends `byte`, waiting until the transmitter can take it.
    pub(crate) fn write_byte(&self, byte: u8) {
        while self.read(LSR) & LSR_THR_EMPTY == 0 {
            core::hint::spin_loop();
        }
        self.write(DATA, byte);
    }

    fn read(&self, register: u16) -> u8 {
        // SAFETY: `new`'s caller gave this driver the ports from `base` to
        // `base + 8`, and every register offset is below 8.
        unsafe { inb(self.base + register) }
    }

    fn write(&self, register: u16, value: u8) {
        // SAFETY: as in `read`.
        unsafe { outb(self.base + register, value) };
    }
}
