//! Driver lifecycle management trait.
//!
//! [`ManagedDriver`] provides optional lifecycle hooks for drivers that support
//! suspend, resume, and orderly shutdown. Uses `async_fn_in_dyn_trait` for
//! dynamic dispatch of async lifecycle methods.

use super::error::DriverError;

/// Lifecycle trait for managed drivers.
///
/// Drivers that support power management or orderly shutdown implement this
/// trait. The kernel calls these methods during system state transitions.
///
/// State machine: `Probing → Active → Suspended ↔ Active → Shutdown`
/// (also `Probing → Failed`).
///
/// All methods have default implementations that return `Unsupported`,
/// so drivers need only override what they support.
pub trait ManagedDriver: Send + Sync {
    /// Suspends the driver, releasing hardware resources that can be
    /// re-acquired on resume.
    ///
    /// Returns `Err(DriverError::Unsupported)` by default.
    fn suspend(&self) -> Result<(), DriverError> {
        Err(DriverError::Unsupported)
    }

    /// Resumes the driver from a suspended state.
    ///
    /// Returns `Err(DriverError::Unsupported)` by default.
    fn resume(&self) -> Result<(), DriverError> {
        Err(DriverError::Unsupported)
    }

    /// Performs an orderly shutdown of the driver.
    ///
    /// Called during system shutdown. Drivers should flush buffers,
    /// disable interrupts, and release DMA resources.
    fn shutdown(&self) {}
}
