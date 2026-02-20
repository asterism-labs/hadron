//! Async PS/2 keyboard driver.
//!
//! Wraps the sync [`I8042`] hardware accessor with an [`IrqLine`] to provide
//! interrupt-driven async key event reads via the [`KeyboardDevice`] trait.

use core::sync::atomic::{AtomicBool, Ordering};

use hadron_kernel::driver_api::capability::IrqCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::input::{KeyCode, KeyEvent, KeyboardDevice};

use crate::input::i8042::{self, I8042};
use hadron_kernel::drivers::irq::IrqLine;

/// Async PS/2 keyboard: composes an [`I8042`] with an [`IrqLine`] for
/// interrupt-driven scancode reading.
pub struct AsyncKeyboard {
    i8042: I8042,
    irq: IrqLine,
    /// Tracks whether the next scancode is an extended (0xE0) sequence.
    extended: AtomicBool,
}

impl AsyncKeyboard {
    /// Creates a new async keyboard driver.
    ///
    /// Binds ISA IRQ 1 to the wakeup handler and unmasks the I/O APIC entry.
    ///
    /// # Errors
    ///
    /// Returns a [`DriverError`] if the IRQ cannot be bound or the I/O APIC
    /// is not initialized.
    pub fn new(irq_cap: &IrqCapability) -> Result<Self, DriverError> {
        let i8042 = I8042::new();

        // 1. Bind IRQ handler to vector.
        let irq = IrqLine::bind_isa(1, irq_cap)?;

        // 2. Unmask the I/O APIC entry for IRQ 1.
        irq_cap.unmask_irq(1)?;

        hadron_kernel::kprintln!(
            "AsyncKeyboard: bound to vector {}, IRQ 1 unmasked",
            irq.vector()
        );

        Ok(Self {
            i8042,
            irq,
            extended: AtomicBool::new(false),
        })
    }

    /// Tries to decode one key event from the scancode stream.
    fn try_decode(&self) -> Option<KeyEvent> {
        let scancode = self.i8042.try_read_keyboard()?;

        // 0xE0 prefix — next byte is an extended scancode.
        if scancode == 0xE0 {
            self.extended.store(true, Ordering::Relaxed);
            return None;
        }

        let pressed = !i8042::is_release(scancode);
        let key = if self.extended.swap(false, Ordering::Relaxed) {
            i8042::extended_scancode_to_keycode(scancode)
                .unwrap_or(KeyCode::Unknown(scancode & 0x7F))
        } else {
            i8042::scancode_to_keycode(scancode).unwrap_or(KeyCode::Unknown(scancode & 0x7F))
        };

        Some(KeyEvent { key, pressed })
    }
}

impl KeyboardDevice for AsyncKeyboard {
    async fn read_event(&self) -> Result<KeyEvent, DriverError> {
        loop {
            if let Some(event) = self.try_decode() {
                return Ok(event);
            }
            // Sleep until keyboard IRQ fires.
            self.irq.wait().await;
        }
    }

    fn event_available(&self) -> bool {
        self.i8042.keyboard_data_available()
    }
}
