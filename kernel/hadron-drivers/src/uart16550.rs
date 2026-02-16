//! UART 16550 serial port driver.
//!
//! Provides a [`Uart16550`] type that implements [`core::fmt::Write`] for
//! formatted text output over a serial port. Supports configurable baud rates,
//! loopback self-test during initialization, and both blocking read/write.

use core::fmt;

use bitflags::bitflags;
use hadron_core::arch::x86_64::Port;

// ---------------------------------------------------------------------------
// Register offsets
// ---------------------------------------------------------------------------

/// Register offsets from the UART base address.
mod reg {
    /// Transmit Holding Register (write, DLAB=0).
    pub const THR: u16 = 0;
    /// Receive Buffer Register (read, DLAB=0).
    pub const RBR: u16 = 0;
    /// Divisor Latch Low byte (DLAB=1).
    pub const DLL: u16 = 0;
    /// Interrupt Enable Register (DLAB=0).
    pub const IER: u16 = 1;
    /// Divisor Latch High byte (DLAB=1).
    pub const DLM: u16 = 1;
    /// Interrupt Identification Register (read).
    pub const IIR: u16 = 2;
    /// FIFO Control Register (write).
    pub const FCR: u16 = 2;
    /// Line Control Register.
    pub const LCR: u16 = 3;
    /// Modem Control Register.
    pub const MCR: u16 = 4;
    /// Line Status Register.
    pub const LSR: u16 = 5;
    /// Modem Status Register.
    #[allow(dead_code)]
    pub const MSR: u16 = 6;
    /// Scratch Register.
    #[allow(dead_code)]
    pub const SCR: u16 = 7;
}

// ---------------------------------------------------------------------------
// Bitflag types
// ---------------------------------------------------------------------------

bitflags! {
    /// Interrupt Enable Register bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Ier: u8 {
        /// Enable Received Data Available interrupt.
        const DATA_AVAILABLE    = 1 << 0;
        /// Enable Transmitter Holding Register Empty interrupt.
        const THR_EMPTY         = 1 << 1;
        /// Enable Receiver Line Status interrupt.
        const LINE_STATUS       = 1 << 2;
        /// Enable Modem Status interrupt.
        const MODEM_STATUS      = 1 << 3;
    }
}

bitflags! {
    /// FIFO Control Register bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Fcr: u8 {
        /// Enable FIFOs.
        const ENABLE            = 1 << 0;
        /// Clear receive FIFO.
        const CLEAR_RX          = 1 << 1;
        /// Clear transmit FIFO.
        const CLEAR_TX          = 1 << 2;
        /// Trigger level: 1 byte.
        const TRIGGER_1         = 0b00 << 6;
        /// Trigger level: 4 bytes.
        const TRIGGER_4         = 0b01 << 6;
        /// Trigger level: 8 bytes.
        const TRIGGER_8         = 0b10 << 6;
        /// Trigger level: 14 bytes.
        const TRIGGER_14        = 0b11 << 6;
    }
}

bitflags! {
    /// Line Control Register bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Lcr: u8 {
        /// Word length bit 0.
        const WORD_LEN_0        = 1 << 0;
        /// Word length bit 1.
        const WORD_LEN_1        = 1 << 1;
        /// Extra stop bit.
        const STOP_BIT          = 1 << 2;
        /// Parity enable.
        const PARITY_ENABLE     = 1 << 3;
        /// Even parity.
        const EVEN_PARITY       = 1 << 4;
        /// Stick parity.
        const STICK_PARITY      = 1 << 5;
        /// Set break.
        const BREAK             = 1 << 6;
        /// Divisor Latch Access Bit.
        const DLAB              = 1 << 7;

        /// 8 data bits, no parity, 1 stop bit.
        const EIGHT_N_ONE = Self::WORD_LEN_0.bits() | Self::WORD_LEN_1.bits();
    }
}

bitflags! {
    /// Modem Control Register bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Mcr: u8 {
        /// Data Terminal Ready.
        const DTR               = 1 << 0;
        /// Request To Send.
        const RTS               = 1 << 1;
        /// Auxiliary output 1.
        const OUT1              = 1 << 2;
        /// Auxiliary output 2 (enables IRQ in PC-compatible UARTs).
        const OUT2              = 1 << 3;
        /// Loopback mode.
        const LOOPBACK          = 1 << 4;
    }
}

bitflags! {
    /// Line Status Register bits.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Lsr: u8 {
        /// Data ready (received data available).
        const DATA_READY        = 1 << 0;
        /// Overrun error.
        const OVERRUN_ERROR     = 1 << 1;
        /// Parity error.
        const PARITY_ERROR      = 1 << 2;
        /// Framing error.
        const FRAMING_ERROR     = 1 << 3;
        /// Break indicator.
        const BREAK_INDICATOR   = 1 << 4;
        /// Transmit Holding Register empty.
        const THR_EMPTY         = 1 << 5;
        /// Transmitter empty (both THR and shift register).
        const TRANSMITTER_EMPTY = 1 << 6;
        /// Error in received FIFO.
        const FIFO_ERROR        = 1 << 7;
    }
}

// ---------------------------------------------------------------------------
// Baud rate
// ---------------------------------------------------------------------------

/// Baud rate selection for UART initialization.
///
/// The discriminant is the divisor value for the UART's clock (1.8432 MHz),
/// so conversion to divisor is zero-cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum BaudRate {
    /// 115200 baud (divisor = 1).
    Baud115200 = 1,
    /// 57600 baud (divisor = 2).
    Baud57600 = 2,
    /// 38400 baud (divisor = 3).
    Baud38400 = 3,
    /// 19200 baud (divisor = 6).
    Baud19200 = 6,
    /// 9600 baud (divisor = 12).
    Baud9600 = 12,
}

impl BaudRate {
    /// Returns the divisor value for this baud rate.
    #[inline]
    pub const fn divisor(self) -> u16 {
        self as u16
    }
}

// ---------------------------------------------------------------------------
// COM port constants
// ---------------------------------------------------------------------------

/// Standard COM1 base I/O port address.
pub const COM1: u16 = 0x3F8;
/// Standard COM2 base I/O port address.
pub const COM2: u16 = 0x2F8;
/// Standard COM3 base I/O port address.
pub const COM3: u16 = 0x3E8;
/// Standard COM4 base I/O port address.
pub const COM4: u16 = 0x2E8;

// ---------------------------------------------------------------------------
// InitError
// ---------------------------------------------------------------------------

/// Error returned when UART initialization fails (loopback self-test).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitError;

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UART 16550 initialization failed (loopback self-test)")
    }
}

// ---------------------------------------------------------------------------
// Uart16550
// ---------------------------------------------------------------------------

/// A UART 16550 serial port identified by its base I/O address.
///
/// This type is `Copy` and carries no state beyond the port address, so it can
/// be freely constructed on the stack (e.g., in a panic handler) without
/// initialization overhead. The UART hardware retains its configuration from
/// the last [`init`](Self::init) call.
#[derive(Debug, Clone, Copy)]
pub struct Uart16550 {
    base: u16,
}

impl Uart16550 {
    /// Creates a new `Uart16550` handle. Does **not** touch hardware.
    #[must_use]
    pub const fn new(base: u16) -> Self {
        Self { base }
    }

    /// Returns a [`Port<u8>`] for the register at the given offset.
    #[inline]
    const fn port(&self, offset: u16) -> Port<u8> {
        Port::new(self.base + offset)
    }

    /// Programs the UART with the given baud rate and 8N1 line settings.
    ///
    /// Performs a loopback self-test to verify the hardware is present and
    /// functioning. Returns [`InitError`] if the self-test fails.
    ///
    /// # Safety
    ///
    /// Must only be called once per port, before any concurrent access.
    /// The caller must ensure `self.base` refers to a valid 16550 UART.
    pub unsafe fn init(&self, baud: BaudRate) -> Result<(), InitError> {
        let divisor = baud.divisor();

        unsafe {
            // 1. Disable all interrupts.
            self.port(reg::IER).write(0x00);

            // 2. Set DLAB, write divisor.
            self.port(reg::LCR).write(Lcr::DLAB.bits());
            self.port(reg::DLL).write(divisor as u8);
            self.port(reg::DLM).write((divisor >> 8) as u8);

            // 3. 8N1, clears DLAB.
            self.port(reg::LCR).write(Lcr::EIGHT_N_ONE.bits());

            // 4. Enable + clear FIFOs, 14-byte trigger.
            self.port(reg::FCR).write(
                (Fcr::ENABLE | Fcr::CLEAR_RX | Fcr::CLEAR_TX | Fcr::TRIGGER_14).bits(),
            );

            // 5. DTR + RTS + OUT2.
            self.port(reg::MCR)
                .write((Mcr::DTR | Mcr::RTS | Mcr::OUT2).bits());

            // 6. Loopback self-test.
            self.port(reg::MCR)
                .write((Mcr::DTR | Mcr::RTS | Mcr::OUT2 | Mcr::LOOPBACK).bits());
            self.port(reg::THR).write(0xAE);

            if self.port(reg::RBR).read() != 0xAE {
                return Err(InitError);
            }

            // 7. Restore normal operation (disable loopback).
            self.port(reg::MCR)
                .write((Mcr::DTR | Mcr::RTS | Mcr::OUT2).bits());
        }

        Ok(())
    }

    /// Writes a single byte, busy-waiting until the transmit buffer is empty.
    pub fn write_byte(&self, byte: u8) {
        unsafe {
            while !Lsr::from_bits_truncate(self.port(reg::LSR).read()).contains(Lsr::THR_EMPTY) {
                core::hint::spin_loop();
            }
            self.port(reg::THR).write(byte);
        }
    }

    /// Reads a single byte, busy-waiting until data is available.
    pub fn read_byte(&self) -> u8 {
        unsafe {
            while !self.data_available() {
                core::hint::spin_loop();
            }
            self.port(reg::RBR).read()
        }
    }

    /// Returns `true` if there is data available to read (non-blocking).
    #[must_use]
    pub fn data_available(&self) -> bool {
        self.line_status().contains(Lsr::DATA_READY)
    }

    /// Returns `true` if the transmit holding register is empty (non-blocking).
    #[must_use]
    pub fn can_write(&self) -> bool {
        self.line_status().contains(Lsr::THR_EMPTY)
    }

    /// Returns the current Line Status Register value.
    #[must_use]
    pub fn line_status(&self) -> Lsr {
        unsafe { Lsr::from_bits_truncate(self.port(reg::LSR).read()) }
    }

    /// Enables the Received Data Available interrupt (IER bit 0).
    ///
    /// # Safety
    ///
    /// The caller must ensure an interrupt handler is registered for this
    /// UART's IRQ vector before enabling, and that the I/O APIC entry is
    /// properly configured.
    pub unsafe fn enable_rx_interrupt(&self) {
        unsafe {
            let ier = self.port(reg::IER).read();
            self.port(reg::IER).write(ier | Ier::DATA_AVAILABLE.bits());
        }
    }

    /// Disables the Received Data Available interrupt (clears IER bit 0).
    pub fn disable_rx_interrupt(&self) {
        unsafe {
            let ier = self.port(reg::IER).read();
            self.port(reg::IER)
                .write(ier & !Ier::DATA_AVAILABLE.bits());
        }
    }

    /// Non-blocking read: returns `Some(byte)` if data is available, `None` otherwise.
    #[must_use]
    pub fn try_read_byte(&self) -> Option<u8> {
        if self.data_available() {
            Some(unsafe { self.port(reg::RBR).read() })
        } else {
            None
        }
    }

    /// Reads the Interrupt Identification Register.
    ///
    /// Reading IIR acknowledges and clears pending interrupt conditions.
    #[must_use]
    pub fn interrupt_id(&self) -> u8 {
        unsafe { self.port(reg::IIR).read() }
    }

    /// Returns the base I/O port address.
    #[must_use]
    pub const fn base(&self) -> u16 {
        self.base
    }
}

impl fmt::Write for Uart16550 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Driver registration
// ---------------------------------------------------------------------------

// Platform driver entry for the 16550 UART.
// Matched by compatible string "ns16550". The actual async setup happens
// in hadron-kernel's AsyncSerial; this entry declares the driver's
// existence to the registry.
#[cfg(target_os = "none")]
hadron_driver_api::platform_driver_entry!(
    UART16550_DRIVER,
    hadron_driver_api::registration::PlatformDriverEntry {
        name: "uart16550",
        compatible: "ns16550",
        init: uart16550_platform_init,
    }
);

#[cfg(target_os = "none")]
fn uart16550_platform_init(
    _services: &'static dyn hadron_driver_api::services::KernelServices,
) -> Result<(), hadron_driver_api::error::DriverError> {
    // Platform init hook — currently a no-op.
    // The actual UART setup is handled by AsyncSerial.
    Ok(())
}
