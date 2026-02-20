//! Kernel-side implementation of the driver services trait.
//!
//! Bridges driver requests to kernel infrastructure: interrupt registration,
//! I/O APIC management, timer access, and device registration.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;

use crate::addr::PhysAddr;
use crate::driver_api::dyn_dispatch::DynBlockDevice;
use crate::driver_api::error::DriverError;
use crate::driver_api::framebuffer::Framebuffer;
use crate::driver_api::resource::MmioRegion;
use crate::driver_api::services::KernelServices;

/// Hadron kernel's implementation of [`KernelServices`].
///
/// Translates trait method calls into direct kernel function calls, keeping
/// all kernel internals hidden from driver crates.
pub struct HadronKernelServices;

impl KernelServices for HadronKernelServices {
    fn register_irq_handler(&self, vector: u8, handler: fn(u8)) -> Result<(), DriverError> {
        crate::arch::interrupts::register_handler(vector, handler)
            .map_err(interrupt_error_to_driver_error)
    }

    fn unregister_irq_handler(&self, vector: u8) {
        crate::arch::interrupts::unregister_handler(vector);
    }

    fn isa_irq_vector(&self, irq: u8) -> u8 {
        crate::arch::interrupts::vectors::isa_irq_vector(irq)
    }

    fn alloc_vector(&self) -> Result<u8, DriverError> {
        crate::arch::interrupts::alloc_vector().map_err(interrupt_error_to_driver_error)
    }

    fn unmask_irq(&self, _isa_irq: u8) -> Result<(), DriverError> {
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::acpi::with_io_apic(|ioapic| ioapic.unmask(_isa_irq))
                .ok_or(DriverError::InitFailed)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Err(DriverError::InitFailed)
        }
    }

    fn mask_irq(&self, _isa_irq: u8) -> Result<(), DriverError> {
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::acpi::with_io_apic(|ioapic| ioapic.mask(_isa_irq))
                .ok_or(DriverError::InitFailed)
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Err(DriverError::InitFailed)
        }
    }

    fn send_eoi(&self) {
        #[cfg(target_arch = "x86_64")]
        crate::arch::x86_64::acpi::send_lapic_eoi();
    }

    fn timer_ticks(&self) -> u64 {
        #[cfg(target_arch = "x86_64")]
        {
            crate::arch::x86_64::acpi::timer_ticks()
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            0
        }
    }

    fn map_mmio(&self, phys_base: u64, size: u64) -> Result<MmioRegion, DriverError> {
        let phys = PhysAddr::new(phys_base);
        let virt = crate::mm::vmm::map_mmio_region(phys, size);
        // SAFETY: The VMM just mapped this region; phys and virt refer to the
        // same physical memory and the mapping is valid for the kernel's lifetime.
        Ok(unsafe { MmioRegion::new(phys, virt, size) })
    }

    fn alloc_dma_frames(&self, count: usize) -> Result<u64, DriverError> {
        crate::mm::pmm::with_pmm(|pmm| {
            pmm.allocate_frames(count)
                .map(|frame| frame.start_address().as_u64())
                .ok_or(DriverError::IoError)
        })
    }

    unsafe fn free_dma_frames(&self, phys_base: u64, count: usize) {
        use crate::paging::{PhysFrame, Size4KiB};
        let frame = PhysFrame::<Size4KiB>::containing_address(PhysAddr::new(phys_base));
        crate::mm::pmm::with_pmm(|pmm| {
            // SAFETY: Caller guarantees phys_base/count match a prior allocation
            // and no DMA operations reference these frames.
            let _ = unsafe { pmm.deallocate_frames(frame, count) };
        });
    }

    fn phys_to_virt(&self, phys: u64) -> u64 {
        crate::mm::hhdm::phys_to_virt(PhysAddr::new(phys)).as_u64()
    }

    fn enable_bus_mastering(&self, bus: u8, device: u8, function: u8) {
        use crate::pci::cam::PciCam;
        // SAFETY: We are setting standard PCI command register bits for a
        // device that was already discovered during enumeration.
        unsafe {
            let cmd = PciCam::read_u32(bus, device, function, 0x04);
            // Set bit 1 (Memory Space Enable) and bit 2 (Bus Master Enable).
            PciCam::write_u32(bus, device, function, 0x04, cmd | 0x06);
        }
    }

    fn register_framebuffer(&self, name: &str, fb: Arc<dyn Framebuffer>) {
        crate::drivers::device_registry::with_device_registry_mut(|dr| {
            dr.register_framebuffer(name, fb);
        });
    }

    fn register_block_device(&self, name: &str, dev: Box<dyn DynBlockDevice>) {
        crate::drivers::device_registry::with_device_registry_mut(|dr| {
            dr.register_block_device(name, dev);
        });
    }

    fn spawn_task(
        &self,
        name: &'static str,
        future: Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
    ) {
        crate::sched::spawn_background(name, future);
    }
}

/// Global kernel services instance passed to drivers during probe/init.
pub static KERNEL_SERVICES: HadronKernelServices = HadronKernelServices;

/// Maps arch-internal [`InterruptError`] to the driver-facing [`DriverError`].
fn interrupt_error_to_driver_error(e: crate::arch::interrupts::InterruptError) -> DriverError {
    use crate::arch::interrupts::InterruptError;
    match e {
        InterruptError::InvalidVector => DriverError::InitFailed,
        InterruptError::AlreadyRegistered => DriverError::InvalidState,
        InterruptError::VectorExhausted => DriverError::InitFailed,
    }
}
