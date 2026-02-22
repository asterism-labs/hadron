//! AArch64 interrupt management (stub).

use crate::id::{HwIrqVector, IrqVector};

/// Error type for interrupt operations.
#[derive(Debug)]
pub struct InterruptError;

/// Register an interrupt handler for the given vector.
pub fn register_handler(_vector: HwIrqVector, _handler: fn(IrqVector)) -> Result<(), InterruptError> {
    todo!("aarch64 register_handler")
}

/// Unregister an interrupt handler for the given vector.
pub fn unregister_handler(_vector: HwIrqVector) {
    todo!("aarch64 unregister_handler")
}

/// Allocate a free interrupt vector.
pub fn alloc_vector() -> Result<HwIrqVector, InterruptError> {
    todo!("aarch64 alloc_vector")
}

/// Vector constants.
pub mod vectors {
    use crate::id::HwIrqVector;

    /// Return the vector number for a given ISA IRQ.
    pub fn isa_irq_vector(_irq: u8) -> HwIrqVector {
        todo!("aarch64 isa_irq_vector")
    }
}
