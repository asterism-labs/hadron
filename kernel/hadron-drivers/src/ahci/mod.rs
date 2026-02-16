//! AHCI (Advanced Host Controller Interface) SATA driver.
//!
//! Drives the Intel ICH9 AHCI controller (vendor 0x8086, device 0x2922) and
//! any AHCI-compatible controller (class 0x01, subclass 0x06, prog-if 0x01).
//! Implements [`BlockDevice`](hadron_driver_api::block::BlockDevice) for each
//! discovered SATA disk.

extern crate alloc;
use alloc::vec::Vec;

use core::ptr;

use hadron_core::sync::SpinLock;
use hadron_driver_api::block::{BlockDevice, IoError};
use hadron_driver_api::error::DriverError;
use hadron_driver_api::pci::{PciBar, PciDeviceId, PciDeviceInfo};
use hadron_driver_api::services::KernelServices;

pub mod command;
pub mod hba;
pub mod port;
pub mod regs;

use hba::AhciHba;
use port::AhciPort;

// ---------------------------------------------------------------------------
// PCI IDs
// ---------------------------------------------------------------------------

/// Intel ICH9 AHCI controller PCI vendor ID.
const ICH9_AHCI_VENDOR: u16 = 0x8086;
/// Intel ICH9 AHCI controller PCI device ID.
const ICH9_AHCI_DEVICE: u16 = 0x2922;

/// PCI class code for mass storage.
const PCI_CLASS_STORAGE: u8 = 0x01;
/// PCI subclass code for SATA.
const PCI_SUBCLASS_SATA: u8 = 0x06;
/// PCI programming interface for AHCI 1.0.
const PCI_PROGIF_AHCI: u8 = 0x01;

/// Default AHCI ABAR size (mapped region, 4 KiB minimum).
const AHCI_ABAR_MIN_SIZE: u64 = 4096;

/// Page size for DMA allocations.
const PAGE_SIZE: u64 = 4096;

// ---------------------------------------------------------------------------
// AhciDisk — BlockDevice wrapper around a port
// ---------------------------------------------------------------------------

/// A SATA disk backed by an AHCI port, implementing async block I/O.
pub struct AhciDisk {
    /// The AHCI port state.
    port: AhciPort,
    /// The bound IRQ line for async completion notification.
    irq: crate::irq::IrqLine,
    /// Kernel services reference (for DMA allocation).
    services: &'static dyn KernelServices,
}

// SAFETY: AhciDisk is Send+Sync because AhciPort is Send+Sync, IrqLine has
// no interior mutability (just a u8 vector), and services is &'static.
unsafe impl Send for AhciDisk {}
unsafe impl Sync for AhciDisk {}

impl BlockDevice for AhciDisk {
    async fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let identity = self.port.identity.as_ref().ok_or(IoError::NotReady)?;
        let ss = identity.sector_size;

        if buf.len() < ss {
            return Err(IoError::InvalidBuffer);
        }
        if sector >= identity.sector_count {
            return Err(IoError::OutOfRange);
        }

        // Allocate a DMA bounce buffer.
        let dma_phys = self
            .services
            .alloc_dma_frames(1)
            .map_err(|_| IoError::DmaError)?;
        let dma_virt = self.services.phys_to_virt(dma_phys);

        // Zero the DMA buffer.
        // SAFETY: Freshly allocated page.
        unsafe { ptr::write_bytes(dma_virt as *mut u8, 0, PAGE_SIZE as usize) };

        let slot = self.port.alloc_slot()?;
        self.port.setup_read_dma(slot, sector, 1, dma_phys, ss);

        let result = self.port.issue_command_async(slot, &self.irq).await;
        self.port.free_slot(slot);

        if result.is_ok() {
            // Copy from DMA bounce buffer to caller's buffer.
            // SAFETY: dma_virt is valid and contains the read data.
            unsafe {
                ptr::copy_nonoverlapping(dma_virt as *const u8, buf.as_mut_ptr(), ss);
            }
        }

        // Free the DMA bounce buffer.
        // SAFETY: We are done with the DMA buffer.
        unsafe { self.services.free_dma_frames(dma_phys, 1) };

        result
    }

    async fn write_sector(&self, _sector: u64, _buf: &[u8]) -> Result<(), IoError> {
        // Phase 10: write support not yet implemented.
        Err(IoError::NotReady)
    }

    fn sector_size(&self) -> usize {
        self.port
            .identity
            .as_ref()
            .map_or(512, |id| id.sector_size)
    }

    fn sector_count(&self) -> u64 {
        self.port
            .identity
            .as_ref()
            .map_or(0, |id| id.sector_count)
    }
}

// ---------------------------------------------------------------------------
// Global disk registry
// ---------------------------------------------------------------------------

/// Global registry of discovered AHCI disks.
static AHCI_DISKS: SpinLock<Option<Vec<AhciDisk>>> = SpinLock::new(None);

/// Returns the number of registered AHCI disks.
#[must_use]
pub fn disk_count() -> usize {
    AHCI_DISKS
        .lock()
        .as_ref()
        .map_or(0, Vec::len)
}

/// Executes a closure with a reference to the disk at `index`.
pub fn with_disk<R>(index: usize, f: impl FnOnce(&AhciDisk) -> R) -> Option<R> {
    let guard = AHCI_DISKS.lock();
    guard.as_ref().and_then(|disks| disks.get(index).map(f))
}

// ---------------------------------------------------------------------------
// PCI registration
// ---------------------------------------------------------------------------

/// PCI device ID table for AHCI controllers.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 2] = [
    PciDeviceId::new(ICH9_AHCI_VENDOR, ICH9_AHCI_DEVICE),
    PciDeviceId::with_class_progif(PCI_CLASS_STORAGE, PCI_SUBCLASS_SATA, PCI_PROGIF_AHCI),
];

#[cfg(target_os = "none")]
hadron_driver_api::pci_driver_entry!(
    AHCI_PCI_DRIVER,
    hadron_driver_api::registration::PciDriverEntry {
        name: "ahci",
        id_table: &ID_TABLE,
        probe: ahci_probe,
    }
);

/// PCI probe function for AHCI controllers.
#[cfg(target_os = "none")]
fn ahci_probe(
    info: &PciDeviceInfo,
    services: &'static dyn KernelServices,
) -> Result<(), DriverError> {
    hadron_core::kinfo!(
        "AHCI: probing {:04x}:{:04x} at {}",
        info.vendor_id,
        info.device_id,
        info.address
    );

    // BAR5 = ABAR (AHCI Base Memory Register).
    let (abar_phys, abar_size) = match info.bars[5] {
        PciBar::Memory { base, size, .. } => (base, size.max(AHCI_ABAR_MIN_SIZE)),
        _ => {
            hadron_core::kwarn!("AHCI: BAR5 is not a memory BAR");
            return Err(DriverError::InitFailed);
        }
    };

    // Enable bus mastering + memory space.
    services.enable_bus_mastering(
        info.address.bus,
        info.address.device,
        info.address.function,
    );

    // Map ABAR.
    let mmio = services.map_mmio(abar_phys, abar_size)?;

    // Initialize HBA.
    // SAFETY: mmio.virt_base() points to the mapped AHCI ABAR.
    let hba = unsafe { AhciHba::new(mmio.virt_base()) };
    hba.enable();

    let (major, minor) = hba.version();
    hadron_core::kinfo!("AHCI: version {}.{}", major, minor);

    // Bind IRQ line for async completion.
    let _irq = crate::irq::IrqLine::bind_isa(info.interrupt_line, services)
        .map_err(|_| DriverError::InitFailed)?;

    // Unmask the IRQ.
    services
        .unmask_irq(info.interrupt_line)
        .map_err(|_| DriverError::InitFailed)?;

    // Enumerate ports.
    let pi = hba.ports_implemented();
    let mut disks = Vec::new();

    for port_num in 0..32u8 {
        if pi & (1 << port_num) == 0 {
            continue;
        }

        hadron_core::kdebug!("AHCI: checking port {}", port_num);

        if let Some(port) = AhciPort::init(&hba, port_num, services) {
            if port.identity.is_some() {
                hadron_core::kinfo!("AHCI: port {} has device", port_num);

                // Clone the IRQ binding for each disk.
                // All ports on the same HBA share the same IRQ.
                let disk_irq = crate::irq::IrqLine::bind_isa(info.interrupt_line, services)
                    .unwrap_or_else(|_| {
                        // If we can't bind a second time (already registered), reuse
                        // by creating a new IrqLine that references the same vector.
                        crate::irq::IrqLine::from_vector(services.isa_irq_vector(info.interrupt_line))
                    });

                disks.push(AhciDisk {
                    port,
                    irq: disk_irq,
                    services,
                });
            }
        }
    }

    if disks.is_empty() {
        hadron_core::kwarn!("AHCI: no devices found on any port");
    } else {
        hadron_core::kinfo!("AHCI: {} disk(s) registered", disks.len());
    }

    let mut global = AHCI_DISKS.lock();
    *global = Some(disks);

    Ok(())
}
