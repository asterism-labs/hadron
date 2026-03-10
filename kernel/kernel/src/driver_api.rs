//! Driver API trait definitions.
//!
//! Defines hardware abstraction traits for kernel infrastructure drivers.
//! Implementations live in arch-specific hw modules.

/// A hardware clock source that provides monotonic time readings.
pub trait ClockSource {
    /// Returns the current time in nanoseconds since the clock was enabled.
    fn read_nanos(&self) -> u64;
}
