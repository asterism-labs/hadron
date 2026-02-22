//! Async serial port driver.
//!
//! Wraps the sync [`Uart16550`] hardware accessor with an [`IrqLine`] to provide
//! interrupt-driven async reads via the [`SerialPort`] trait. TX remains synchronous
//! (the async signature permits future flow-control implementations).

use hadron_kernel::driver_api::capability::IrqCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::serial::SerialPort;

use crate::uart16550::Uart16550;
use hadron_kernel::drivers::irq::IrqLine;

/// Async serial port: composes a [`Uart16550`] with an [`IrqLine`] for
/// interrupt-driven RX.
pub struct AsyncSerial {
    uart: Uart16550,
    irq: IrqLine,
}

impl AsyncSerial {
    /// Creates a new async serial port.
    ///
    /// Binds the ISA IRQ to the wakeup handler, unmasks the I/O APIC entry,
    /// and enables the UART RX interrupt.
    ///
    /// # Errors
    ///
    /// Returns a [`DriverError`] if the IRQ cannot be bound or the I/O APIC
    /// is not initialized.
    pub fn new(uart: Uart16550, isa_irq: u8, irq_cap: &IrqCapability) -> Result<Self, DriverError> {
        // 1. Bind IRQ handler to vector.
        let irq = IrqLine::bind_isa(isa_irq, irq_cap)?;

        // 2. Unmask the I/O APIC entry for this IRQ.
        irq_cap.unmask_irq(isa_irq)?;

        // 3. Enable UART RX interrupt (IER bit 0).
        // SAFETY: We just registered a handler and unmasked the I/O APIC entry.
        unsafe { uart.enable_rx_interrupt() };

        hadron_kernel::kprintln!(
            "AsyncSerial: COM port {:#x} bound to vector {}, IRQ {} unmasked",
            uart.base(),
            irq.vector(),
            isa_irq
        );

        Ok(Self { uart, irq })
    }
}

impl SerialPort for AsyncSerial {
    async fn write_byte(&self, byte: u8) -> Result<(), DriverError> {
        // TX is synchronous — Uart16550::write_byte busy-waits for THR empty,
        // which is effectively instant at current baud rates.
        self.uart.write_byte(byte);
        Ok(())
    }

    async fn read_byte(&self) -> Result<u8, DriverError> {
        loop {
            // Check if data is already available (e.g., from FIFO).
            if let Some(byte) = self.uart.try_read_byte() {
                return Ok(byte);
            }
            // Sleep until the RX interrupt fires.
            self.irq.wait().await;
        }
    }

    fn data_available(&self) -> bool {
        self.uart.data_available()
    }

    fn can_write(&self) -> bool {
        self.uart.can_write()
    }
}
