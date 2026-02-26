//! Base driver trait and metadata types.

/// The type of hardware a driver manages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverType {
    /// Serial port / UART.
    Serial,
    /// Framebuffer / display output.
    Framebuffer,
    /// Block storage device.
    Block,
    /// Network interface.
    Network,
    /// Input device (keyboard, mouse).
    Input,
    /// Timer / clock source.
    Timer,
    /// Interrupt controller (APIC, GIC).
    InterruptController,
    /// Platform / system device.
    Platform,
}

/// Static metadata describing a driver.
#[derive(Debug, Clone, Copy)]
pub struct DriverInfo {
    /// Short name of the driver (e.g. "uart16550").
    pub name: &'static str,
    /// The type of hardware this driver manages.
    pub driver_type: DriverType,
    /// Human-readable description.
    pub description: &'static str,
}

/// The lifecycle state of a registered driver instance.
///
/// Tracked externally by the driver registry, not by individual drivers.
///
/// State machine: `Probing → Active → Suspended ↔ Active → Shutdown`
/// (also `Probing → Failed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverState {
    /// Driver is registered but not yet probed.
    Registered,
    /// Driver probe is in progress.
    Probing,
    /// Driver has been probed and is active.
    Active,
    /// Driver has been suspended (can be resumed).
    Suspended,
    /// Driver has been shut down.
    Shutdown,
    /// Driver probe or operation failed.
    Failed,
}

/// Base trait that all drivers implement to provide identity and metadata.
pub trait Driver {
    /// Returns static information about this driver.
    fn info(&self) -> DriverInfo;
}
