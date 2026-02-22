//! Probe contexts for driver initialization.
//!
//! The kernel constructs a [`PciProbeContext`] or [`PlatformProbeContext`]
//! with exactly the capabilities a driver needs, then passes it to the
//! driver's probe/init function.

use super::capability::{
    DmaCapability, IrqCapability, MmioCapability, PciConfigCapability, TaskSpawner, TimerCapability,
};
use super::pci::PciDeviceInfo;

/// Probe context for PCI drivers.
///
/// Contains the discovered device info plus all capability tokens the
/// driver needs for initialization.
pub struct PciProbeContext {
    /// Information about the matched PCI device.
    pub device: PciDeviceInfo,
    /// PCI configuration space access, scoped to this device's BDF.
    pub pci_config: PciConfigCapability,
    /// Interrupt management capability.
    pub irq: IrqCapability,
    /// MMIO mapping capability.
    pub mmio: MmioCapability,
    /// DMA memory allocation capability.
    pub dma: DmaCapability,
    /// Task spawning capability.
    pub spawner: TaskSpawner,
    /// Timer access capability.
    pub timer: TimerCapability,
}

/// Probe context for platform drivers.
///
/// Platform drivers don't need PCI config access or device info, but do
/// need interrupt, MMIO, task spawning, and timer capabilities.
pub struct PlatformProbeContext {
    /// Interrupt management capability.
    pub irq: IrqCapability,
    /// MMIO mapping capability.
    pub mmio: MmioCapability,
    /// Task spawning capability.
    pub spawner: TaskSpawner,
    /// Timer access capability.
    pub timer: TimerCapability,
}

/// Constructs a [`PciProbeContext`] for a discovered PCI device.
///
/// Called by the kernel's driver matching logic before invoking a PCI
/// driver's probe function.
#[cfg(target_os = "none")]
pub(crate) fn pci_probe_context(info: &PciDeviceInfo) -> PciProbeContext {
    PciProbeContext {
        device: info.clone(),
        pci_config: PciConfigCapability::new(
            info.address.bus,
            info.address.device,
            info.address.function,
        ),
        irq: IrqCapability::new(),
        mmio: MmioCapability::new(),
        dma: DmaCapability::new(),
        spawner: TaskSpawner::new(),
        timer: TimerCapability::new(),
    }
}

/// Constructs a [`PlatformProbeContext`] for a platform device.
///
/// Called by the kernel's driver matching logic before invoking a platform
/// driver's init function.
#[cfg(target_os = "none")]
pub(crate) fn platform_probe_context() -> PlatformProbeContext {
    PlatformProbeContext {
        irq: IrqCapability::new(),
        mmio: MmioCapability::new(),
        spawner: TaskSpawner::new(),
        timer: TimerCapability::new(),
    }
}
