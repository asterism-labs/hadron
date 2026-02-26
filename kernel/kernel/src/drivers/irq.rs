//! IRQ-to-async bridge.
//!
//! Provides [`IrqLine`] which binds an interrupt vector to a [`WaitQueue`],
//! allowing async tasks to sleep until a hardware interrupt fires.

use core::future::Future;

use crate::driver_api::capability::IrqCapability;
use crate::driver_api::error::DriverError;
use crate::id::{HwIrqVector, IrqVector};
use crate::sync::WaitQueue;

/// Number of IRQ wait queues covering vectors 32-255 (ISA + MSI-X).
const MAX_IRQ_LINES: usize = 224;

/// One wait queue per vector (32-255). Index = vector - 32.
static IRQ_WAITQUEUES: [WaitQueue; MAX_IRQ_LINES] = {
    const INIT: WaitQueue = WaitQueue::new();
    [INIT; MAX_IRQ_LINES]
};

/// Generic IRQ handler that wakes all tasks waiting on the corresponding vector.
fn irq_wakeup_handler(vector: IrqVector) {
    let idx = (vector.as_u8() - 32) as usize;
    if idx < MAX_IRQ_LINES {
        IRQ_WAITQUEUES[idx].wake_all();
    }
}

/// An interrupt line bound to an async wait queue.
///
/// Created via [`IrqLine::bind`], which registers the wakeup handler for the
/// given vector. Async tasks call [`wait`](IrqLine::wait) to sleep until the
/// next interrupt on this line.
pub struct IrqLine {
    vector: HwIrqVector,
}

impl IrqLine {
    /// Binds a wakeup handler to the given hardware interrupt vector.
    ///
    /// The vector should be an ISA IRQ vector (32-47) or a dynamically
    /// allocated vector. Returns an error if the vector is already in use.
    pub fn bind(vector: HwIrqVector, irq_cap: &IrqCapability) -> Result<Self, DriverError> {
        irq_cap.register_handler(vector, irq_wakeup_handler)?;
        Ok(Self { vector })
    }

    /// Binds a wakeup handler to the ISA IRQ number (0-15).
    ///
    /// Convenience wrapper that converts the IRQ number to a vector.
    pub fn bind_isa(irq: u8, irq_cap: &IrqCapability) -> Result<Self, DriverError> {
        Self::bind(irq_cap.isa_irq_vector(irq), irq_cap)
    }

    /// Creates an `IrqLine` for a vector whose handler is already registered.
    ///
    /// Use this when multiple consumers share the same IRQ vector (e.g.,
    /// multiple AHCI ports on the same HBA).
    #[must_use]
    pub const fn from_vector(vector: HwIrqVector) -> Self {
        Self { vector }
    }

    /// Returns the bound hardware interrupt vector.
    #[must_use]
    pub const fn vector(&self) -> HwIrqVector {
        self.vector
    }

    /// Returns a future that completes when the next interrupt fires on this line.
    pub fn wait(&self) -> impl Future<Output = ()> + '_ {
        IRQ_WAITQUEUES[self.vector.table_index()].wait()
    }
}
