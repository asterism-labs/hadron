//! Typed capability tokens for driver-kernel interaction.
//!
//! Instead of a monolithic `KernelServices` trait, drivers receive only the
//! capabilities they need. Each capability type has `pub(crate)` constructors
//! so only the kernel can mint them — drivers cannot forge access.

extern crate alloc;

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;

use super::error::DriverError;
use super::resource::MmioRegion;
use crate::id::IrqVector;

// ---------------------------------------------------------------------------
// IrqCapability
// ---------------------------------------------------------------------------

/// Capability token for interrupt management.
///
/// Allows a driver to register/unregister interrupt handlers, query ISA IRQ
/// vectors, allocate dynamic vectors, and mask/unmask I/O APIC entries.
pub struct IrqCapability {
    _private: (),
}

impl IrqCapability {
    /// Creates a new IRQ capability.
    ///
    /// Only the kernel can construct this (unforgeable by driver crates).
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    /// Registers an interrupt handler for the given vector.
    pub fn register_handler(
        &self,
        vector: IrqVector,
        handler: fn(IrqVector),
    ) -> Result<(), DriverError> {
        #[cfg(target_os = "none")]
        {
            crate::arch::interrupts::register_handler(vector, handler)
                .map_err(interrupt_error_to_driver_error)
        }
        #[cfg(not(target_os = "none"))]
        {
            let _ = (vector, handler);
            Err(DriverError::Unsupported)
        }
    }

    /// Unregisters a previously registered interrupt handler.
    pub fn unregister_handler(&self, vector: IrqVector) {
        #[cfg(target_os = "none")]
        crate::arch::interrupts::unregister_handler(vector);
        #[cfg(not(target_os = "none"))]
        let _ = vector;
    }

    /// Returns the interrupt vector for a given ISA IRQ number.
    pub fn isa_irq_vector(&self, irq: u8) -> IrqVector {
        #[cfg(target_os = "none")]
        {
            crate::arch::interrupts::vectors::isa_irq_vector(irq)
        }
        #[cfg(not(target_os = "none"))]
        {
            IrqVector::new(irq + 32)
        }
    }

    /// Allocates a free interrupt vector.
    pub fn alloc_vector(&self) -> Result<IrqVector, DriverError> {
        #[cfg(target_os = "none")]
        {
            crate::arch::interrupts::alloc_vector().map_err(interrupt_error_to_driver_error)
        }
        #[cfg(not(target_os = "none"))]
        Err(DriverError::Unsupported)
    }

    /// Unmasks (enables) an ISA IRQ line in the I/O APIC.
    pub fn unmask_irq(&self, isa_irq: u8) -> Result<(), DriverError> {
        #[cfg(all(target_os = "none", target_arch = "x86_64"))]
        {
            crate::arch::x86_64::acpi::with_io_apic(|ioapic| ioapic.unmask(isa_irq))
                .ok_or(DriverError::InitFailed)
        }
        #[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
        {
            let _ = isa_irq;
            Err(DriverError::InitFailed)
        }
    }

    /// Masks (disables) an ISA IRQ line in the I/O APIC.
    pub fn mask_irq(&self, isa_irq: u8) -> Result<(), DriverError> {
        #[cfg(all(target_os = "none", target_arch = "x86_64"))]
        {
            crate::arch::x86_64::acpi::with_io_apic(|ioapic| ioapic.mask(isa_irq))
                .ok_or(DriverError::InitFailed)
        }
        #[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
        {
            let _ = isa_irq;
            Err(DriverError::InitFailed)
        }
    }

    /// Sends an End-of-Interrupt signal.
    pub fn send_eoi(&self) {
        #[cfg(all(target_os = "none", target_arch = "x86_64"))]
        crate::arch::x86_64::acpi::send_lapic_eoi();
    }
}

// ---------------------------------------------------------------------------
// MmioCapability
// ---------------------------------------------------------------------------

/// Capability token for MMIO mapping.
///
/// Allows a driver to map physical MMIO regions into kernel virtual space
/// and translate physical addresses via the HHDM.
#[derive(Clone, Copy)]
pub struct MmioCapability {
    _private: (),
}

impl MmioCapability {
    /// Creates a new MMIO capability.
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    /// Maps a physical MMIO region into kernel virtual address space.
    pub fn map_mmio(&self, phys_base: u64, size: u64) -> Result<MmioRegion, DriverError> {
        #[cfg(target_os = "none")]
        {
            let phys = crate::addr::PhysAddr::new(phys_base);
            let mapping = crate::mm::vmm::map_mmio_region(phys, size);
            let virt = mapping.virt_base();
            core::mem::forget(mapping); // driver mappings are permanent
            // SAFETY: The VMM just mapped this region; phys and virt refer to the
            // same physical memory and the mapping is valid for the kernel's lifetime.
            Ok(unsafe { MmioRegion::new(phys, virt, size) })
        }
        #[cfg(not(target_os = "none"))]
        {
            let _ = (phys_base, size);
            Err(DriverError::Unsupported)
        }
    }

    /// Converts a physical address to its kernel virtual address via the HHDM.
    pub fn phys_to_virt(&self, phys: u64) -> u64 {
        #[cfg(target_os = "none")]
        {
            crate::mm::hhdm::phys_to_virt(crate::addr::PhysAddr::new(phys)).as_u64()
        }
        #[cfg(not(target_os = "none"))]
        {
            phys
        }
    }
}

// ---------------------------------------------------------------------------
// DmaCapability
// ---------------------------------------------------------------------------

/// Capability token for DMA memory allocation.
///
/// Allows a driver to allocate and free contiguous physical frames for
/// DMA transfers.
#[derive(Clone, Copy)]
pub struct DmaCapability {
    _private: (),
}

impl DmaCapability {
    /// Creates a new DMA capability.
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    /// Allocates `count` contiguous physical frames for DMA use.
    ///
    /// Returns the physical base address of the first frame.
    pub fn alloc_frames(&self, count: usize) -> Result<u64, DriverError> {
        #[cfg(target_os = "none")]
        {
            crate::mm::pmm::with_pmm(|pmm| {
                pmm.allocate_frames(count)
                    .map(|frame| frame.start_address().as_u64())
                    .ok_or(DriverError::IoError)
            })
        }
        #[cfg(not(target_os = "none"))]
        {
            let _ = count;
            Err(DriverError::Unsupported)
        }
    }

    /// Converts a physical address to its kernel virtual address via the HHDM.
    ///
    /// Commonly used after [`alloc_frames`](Self::alloc_frames) to get a
    /// virtual pointer to the allocated DMA memory.
    pub fn phys_to_virt(&self, phys: u64) -> u64 {
        #[cfg(target_os = "none")]
        {
            crate::mm::hhdm::phys_to_virt(crate::addr::PhysAddr::new(phys)).as_u64()
        }
        #[cfg(not(target_os = "none"))]
        {
            phys
        }
    }

    /// Frees DMA frames previously allocated with [`alloc_frames`](Self::alloc_frames).
    ///
    /// # Safety
    ///
    /// The caller must ensure that no DMA operations reference these frames
    /// and that `phys_base` and `count` match a previous allocation.
    pub unsafe fn free_frames(&self, phys_base: u64, count: usize) {
        #[cfg(target_os = "none")]
        {
            use crate::paging::{PhysFrame, Size4KiB};
            let frame =
                PhysFrame::<Size4KiB>::containing_address(crate::addr::PhysAddr::new(phys_base));
            crate::mm::pmm::with_pmm(|pmm| {
                // SAFETY: Caller guarantees phys_base/count match a prior allocation
                // and no DMA operations reference these frames.
                let _ = unsafe { pmm.deallocate_frames(frame, count) };
            });
        }
        #[cfg(not(target_os = "none"))]
        let _ = (phys_base, count);
    }
}

// ---------------------------------------------------------------------------
// PciConfigCapability
// ---------------------------------------------------------------------------

/// Capability token for PCI configuration space access.
///
/// Scoped to a specific BDF (bus/device/function) address — a driver can
/// only access config space for its own device.
pub struct PciConfigCapability {
    bus: u8,
    device: u8,
    function: u8,
}

impl PciConfigCapability {
    /// Creates a new PCI config capability scoped to the given BDF.
    pub(crate) fn new(bus: u8, device: u8, function: u8) -> Self {
        Self {
            bus,
            device,
            function,
        }
    }

    /// Enables PCI bus mastering for this device.
    ///
    /// Sets bits 1 (Memory Space) and 2 (Bus Master) in the PCI Command register.
    pub fn enable_bus_mastering(&self) {
        #[cfg(target_os = "none")]
        {
            use crate::pci::cam::PciCam;
            // SAFETY: We are setting standard PCI command register bits for a
            // device that was already discovered during enumeration.
            unsafe {
                let cmd = PciCam::read_u32(self.bus, self.device, self.function, 0x04);
                // Set bit 1 (Memory Space Enable) and bit 2 (Bus Master Enable).
                PciCam::write_u32(self.bus, self.device, self.function, 0x04, cmd | 0x06);
            }
        }
    }

    /// Reads a 32-bit value from PCI configuration space.
    pub fn read_config_u32(&self, offset: u8) -> u32 {
        #[cfg(target_os = "none")]
        {
            use crate::pci::cam::PciCam;
            // SAFETY: Reading PCI config space for an enumerated device.
            unsafe { PciCam::read_u32(self.bus, self.device, self.function, offset) }
        }
        #[cfg(not(target_os = "none"))]
        {
            let _ = offset;
            0
        }
    }

    /// Writes a 32-bit value to PCI configuration space.
    pub fn write_config_u32(&self, offset: u8, value: u32) {
        #[cfg(target_os = "none")]
        {
            use crate::pci::cam::PciCam;
            // SAFETY: Writing PCI config space for an enumerated device.
            unsafe { PciCam::write_u32(self.bus, self.device, self.function, offset, value) }
        }
        #[cfg(not(target_os = "none"))]
        let _ = (offset, value);
    }

    /// Returns the bus number for this device.
    #[must_use]
    pub fn bus(&self) -> u8 {
        self.bus
    }

    /// Returns the device number for this device.
    #[must_use]
    pub fn device(&self) -> u8 {
        self.device
    }

    /// Returns the function number for this device.
    #[must_use]
    pub fn function(&self) -> u8 {
        self.function
    }
}

// ---------------------------------------------------------------------------
// TaskSpawner
// ---------------------------------------------------------------------------

/// Capability token for spawning async tasks on the kernel executor.
pub struct TaskSpawner {
    _private: (),
}

impl TaskSpawner {
    /// Creates a new task spawner capability.
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    /// Spawns a background async task on the kernel's executor.
    pub fn spawn(
        &self,
        name: &'static str,
        future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
    ) {
        #[cfg(target_os = "none")]
        crate::sched::spawn_background(name, future);
        #[cfg(not(target_os = "none"))]
        let _ = (name, future);
    }
}

// ---------------------------------------------------------------------------
// TimerCapability
// ---------------------------------------------------------------------------

/// Capability token for accessing kernel timers.
#[derive(Clone, Copy)]
pub struct TimerCapability {
    _private: (),
}

impl TimerCapability {
    /// Creates a new timer capability.
    pub(crate) fn new() -> Self {
        Self { _private: () }
    }

    /// Returns the current timer tick count.
    pub fn timer_ticks(&self) -> u64 {
        #[cfg(all(target_os = "none", target_arch = "x86_64"))]
        {
            crate::arch::x86_64::acpi::timer_ticks()
        }
        #[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
        0
    }
}

// ---------------------------------------------------------------------------
// Capability bitmap for runtime auditing
// ---------------------------------------------------------------------------

bitflags::bitflags! {
    /// Bitmap of capabilities a driver has requested.
    ///
    /// Stored in driver entries for runtime auditing and logging.
    /// Compile-time enforcement is handled separately by generated context types.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CapabilityFlags: u32 {
        /// Interrupt management.
        const IRQ          = 1 << 0;
        /// MMIO mapping.
        const MMIO         = 1 << 1;
        /// DMA memory allocation.
        const DMA          = 1 << 2;
        /// PCI configuration space access.
        const PCI_CONFIG   = 1 << 3;
        /// Async task spawning.
        const TASK_SPAWNER = 1 << 4;
        /// Kernel timer access.
        const TIMER        = 1 << 5;
    }
}

impl core::fmt::Display for CapabilityFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        bitflags::parser::to_writer(self, f)
    }
}

// ---------------------------------------------------------------------------
// Compile-time capability access traits
// ---------------------------------------------------------------------------

/// Proof that a driver context grants access to capability `C`.
///
/// Implemented by generated driver context types for each declared capability.
/// Cannot be implemented outside `hadron-kernel` due to the sealed
/// [`CapabilityToken`] bound.
pub trait HasCapability<C: CapabilityToken> {
    /// Returns a reference to the capability token.
    fn get(&self) -> &C;
}

/// Provides the ergonomic `.capability::<T>()` method on all driver contexts.
pub trait CapabilityAccess {
    /// Returns a reference to capability `C` if this context grants it.
    ///
    /// Fails to compile if `C` was not declared in the driver's capabilities list.
    fn capability<C: CapabilityToken>(&self) -> &C
    where
        Self: HasCapability<C>,
    {
        <Self as HasCapability<C>>::get(self)
    }
}

/// Blanket impl — every type gets `.capability::<T>()`.
impl<T> CapabilityAccess for T {}

/// Sealed marker trait for capability types.
///
/// Prevents external crates from implementing [`HasCapability`] for arbitrary types.
pub trait CapabilityToken: sealed::Sealed {}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for IrqCapability {}
impl sealed::Sealed for MmioCapability {}
impl sealed::Sealed for DmaCapability {}
impl sealed::Sealed for PciConfigCapability {}
impl sealed::Sealed for TaskSpawner {}
impl sealed::Sealed for TimerCapability {}

impl CapabilityToken for IrqCapability {}
impl CapabilityToken for MmioCapability {}
impl CapabilityToken for DmaCapability {}
impl CapabilityToken for PciConfigCapability {}
impl CapabilityToken for TaskSpawner {}
impl CapabilityToken for TimerCapability {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Maps arch-internal [`InterruptError`] to the driver-facing [`DriverError`].
#[cfg(target_os = "none")]
fn interrupt_error_to_driver_error(e: crate::arch::interrupts::InterruptError) -> DriverError {
    use crate::arch::interrupts::InterruptError;
    match e {
        InterruptError::InvalidVector => DriverError::InitFailed,
        InterruptError::AlreadyRegistered => DriverError::InvalidState,
        InterruptError::VectorExhausted => DriverError::InitFailed,
    }
}
