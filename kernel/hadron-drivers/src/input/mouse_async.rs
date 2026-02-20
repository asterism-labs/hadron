//! Async PS/2 mouse driver.
//!
//! Wraps the sync [`I8042`] hardware accessor with an [`IrqLine`] to provide
//! interrupt-driven async mouse event reads via the [`MouseDevice`] trait.

use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::input::{MouseDevice, MouseEvent};
use hadron_kernel::driver_api::services::KernelServices;

use crate::input::i8042::{I8042, MousePacket};
use hadron_kernel::drivers::irq::IrqLine;

/// Async PS/2 mouse: composes an [`I8042`] with an [`IrqLine`] for
/// interrupt-driven mouse packet collection.
pub struct AsyncMouse {
    i8042: I8042,
    irq: IrqLine,
}

impl AsyncMouse {
    /// Creates a new async mouse driver.
    ///
    /// Binds ISA IRQ 12 to the wakeup handler and unmasks the I/O APIC entry.
    ///
    /// # Errors
    ///
    /// Returns a [`DriverError`] if the IRQ cannot be bound or the I/O APIC
    /// is not initialized.
    pub fn new(services: &'static dyn KernelServices) -> Result<Self, DriverError> {
        let i8042 = I8042::new();

        // 1. Bind IRQ handler to vector.
        let irq = IrqLine::bind_isa(12, services)?;

        // 2. Unmask the I/O APIC entry for IRQ 12.
        services.unmask_irq(12)?;

        hadron_kernel::kprintln!(
            "AsyncMouse: bound to vector {}, IRQ 12 unmasked",
            irq.vector()
        );

        Ok(Self { i8042, irq })
    }

    /// Collects a single byte from the mouse port, waiting on IRQ if needed.
    async fn read_mouse_byte(&self) -> u8 {
        loop {
            if let Some(byte) = self.i8042.try_read_mouse() {
                return byte;
            }
            self.irq.wait().await;
        }
    }
}

impl MouseDevice for AsyncMouse {
    async fn read_event(&self) -> Result<MouseEvent, DriverError> {
        // Collect a full 3-byte packet.
        let b0 = self.read_mouse_byte().await;
        let b1 = self.read_mouse_byte().await;
        let b2 = self.read_mouse_byte().await;

        let packet = MousePacket::parse([b0, b1, b2]);

        Ok(MouseEvent {
            dx: packet.dx,
            dy: packet.dy,
            left: packet.left,
            right: packet.right,
            middle: packet.middle,
        })
    }

    fn event_available(&self) -> bool {
        self.i8042.mouse_data_available()
    }
}
