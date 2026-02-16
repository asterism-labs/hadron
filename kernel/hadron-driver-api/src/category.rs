//! Driver category traits defining lifecycle and probe patterns.

use crate::driver::Driver;
use crate::error::DriverError;

/// A platform driver discovered through firmware tables or hard-coded knowledge.
///
/// Platform drivers are the simplest category: they receive pre-allocated
/// resources, probe the hardware, and return a fully initialized driver instance.
///
/// The `Sized` bound enables returning `Self` from `probe()` without boxing.
/// Resources are consumed (moved) to enforce exclusive ownership at the type level.
#[allow(async_fn_in_trait)] // Used only internally; no dyn dispatch needed.
pub trait PlatformDriver: Driver + Sized {
    /// The resource bundle this driver needs to probe (e.g., I/O port ranges, MMIO regions).
    type Resources;

    /// Probes the hardware using the given resources and returns an initialized driver.
    ///
    /// Consumes the resources to enforce exclusive ownership. Async to permit
    /// probe sequences that wait on hardware (e.g., device identification via IRQ).
    ///
    /// # Errors
    ///
    /// Returns [`DriverError`] if the hardware is not present or initialization fails.
    async fn probe(resources: Self::Resources) -> Result<Self, DriverError>;

    /// Shuts down the driver, releasing hardware resources.
    ///
    /// Best-effort: shutdown failures are not actionable, so this returns `()`.
    fn shutdown(&mut self);
}
