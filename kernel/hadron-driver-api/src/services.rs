//! Kernel service contracts for drivers.
//!
//! Drivers use [`KernelServices`] to interact with kernel infrastructure
//! (interrupt handlers, IRQ management, timers) without depending on the
//! kernel crate directly.

use crate::error::DriverError;
use crate::resource::MmioRegion;

/// Trait providing kernel services to drivers.
///
/// Implemented by the kernel and passed to drivers during probe/init, allowing
/// drivers to register interrupt handlers, unmask IRQs, map MMIO regions,
/// allocate DMA memory, and query timers without a direct dependency on
/// kernel internals.
pub trait KernelServices: Send + Sync {
    /// Registers an interrupt handler for the given vector.
    fn register_irq_handler(&self, vector: u8, handler: fn(u8)) -> Result<(), DriverError>;

    /// Unregisters a previously registered interrupt handler.
    fn unregister_irq_handler(&self, vector: u8);

    /// Returns the interrupt vector for a given ISA IRQ number.
    fn isa_irq_vector(&self, irq: u8) -> u8;

    /// Allocates a free interrupt vector.
    fn alloc_vector(&self) -> Result<u8, DriverError>;

    /// Unmasks (enables) an ISA IRQ line in the I/O APIC.
    fn unmask_irq(&self, isa_irq: u8) -> Result<(), DriverError>;

    /// Masks (disables) an ISA IRQ line in the I/O APIC.
    fn mask_irq(&self, isa_irq: u8) -> Result<(), DriverError>;

    /// Sends an End-of-Interrupt signal.
    fn send_eoi(&self);

    /// Returns the current timer tick count.
    fn timer_ticks(&self) -> u64;

    /// Maps a physical MMIO region into kernel virtual address space.
    ///
    /// Returns an [`MmioRegion`] describing the mapped region.
    fn map_mmio(&self, phys_base: u64, size: u64) -> Result<MmioRegion, DriverError>;

    /// Allocates `count` contiguous physical frames for DMA use.
    ///
    /// Returns the physical base address of the first frame.
    fn alloc_dma_frames(&self, count: usize) -> Result<u64, DriverError>;

    /// Frees DMA frames previously allocated with [`alloc_dma_frames`](Self::alloc_dma_frames).
    ///
    /// # Safety
    ///
    /// The caller must ensure that no DMA operations reference these frames
    /// and that `phys_base` and `count` match a previous allocation.
    unsafe fn free_dma_frames(&self, phys_base: u64, count: usize);

    /// Converts a physical address to its kernel virtual address via the HHDM.
    fn phys_to_virt(&self, phys: u64) -> u64;

    /// Enables PCI bus mastering for the device at the given BDF address.
    ///
    /// Sets bits 1 (Memory Space) and 2 (Bus Master) in the PCI Command register.
    fn enable_bus_mastering(&self, bus: u8, device: u8, function: u8);
}
