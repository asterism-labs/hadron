//! AHCI (Advanced Host Controller Interface) SATA driver.
//!
//! Drives the Intel ICH9 AHCI controller (vendor 0x8086, device 0x2922) and
//! any AHCI-compatible controller (class 0x01, subclass 0x06, prog-if 0x01).
//! Implements [`BlockDevice`](hadron_kernel::driver_api::block::BlockDevice) for each
//! discovered SATA disk.

extern crate alloc;

use alloc::vec::Vec;
use core::ptr;

use hadron_kernel::sync::SpinLock;
use hadron_kernel::driver_api::block::{BlockDevice, IoError};
use hadron_kernel::driver_api::capability::DmaCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::pci::{PciBar, PciDeviceId};

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
    irq: hadron_kernel::drivers::irq::IrqLine,
    /// DMA capability for memory allocation.
    dma: DmaCapability,
}

// SAFETY: AhciDisk is Send+Sync because AhciPort is Send+Sync, IrqLine has
// no interior mutability (just a u8 vector), and DmaCapability is Copy.
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
            .dma
            .alloc_frames(1)
            .map_err(|_| IoError::DmaError)?;
        let dma_virt = self.dma.phys_to_virt(dma_phys);

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
        unsafe { self.dma.free_frames(dma_phys, 1) };

        result
    }

    async fn write_sector(&self, _sector: u64, _buf: &[u8]) -> Result<(), IoError> {
        // Phase 10: write support not yet implemented.
        Err(IoError::NotReady)
    }

    fn sector_size(&self) -> usize {
        self.port.identity.as_ref().map_or(512, |id| id.sector_size)
    }

    fn sector_count(&self) -> u64 {
        self.port.identity.as_ref().map_or(0, |id| id.sector_count)
    }
}

/// Counter for assigning unique device names to discovered AHCI disks.
static DISK_INDEX: SpinLock<usize> = SpinLock::new(0);

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
hadron_kernel::pci_driver_entry!(
    AHCI_PCI_DRIVER,
    hadron_kernel::driver_api::registration::PciDriverEntry {
        name: "ahci",
        id_table: &ID_TABLE,
        probe: ahci_probe,
    }
);

/// PCI probe function for AHCI controllers.
#[cfg(target_os = "none")]
fn ahci_probe(
    ctx: hadron_kernel::driver_api::probe_context::PciProbeContext,
) -> Result<hadron_kernel::driver_api::registration::PciDriverRegistration, DriverError> {
    use hadron_kernel::driver_api::device_path::DevicePath;
    use hadron_kernel::driver_api::registration::{DeviceSet, PciDriverRegistration};

    let info = &ctx.device;
    hadron_kernel::kinfo!(
        "AHCI: probing {:04x}:{:04x} at {}",
        info.vendor_id,
        info.device_id,
        info.address
    );

    // BAR5 = ABAR (AHCI Base Memory Register).
    let (abar_phys, abar_size) = match info.bars[5] {
        PciBar::Memory { base, size, .. } => (base, size.max(AHCI_ABAR_MIN_SIZE)),
        _ => {
            hadron_kernel::kwarn!("AHCI: BAR5 is not a memory BAR");
            return Err(DriverError::InitFailed);
        }
    };

    // Enable bus mastering + memory space.
    ctx.pci_config.enable_bus_mastering();

    // Map ABAR.
    let mmio = ctx.mmio.map_mmio(abar_phys, abar_size)?;

    // Initialize HBA.
    // SAFETY: mmio.virt_base() points to the mapped AHCI ABAR.
    let hba = unsafe { AhciHba::new(mmio.virt_base()) };
    hba.enable();

    let (major, minor) = hba.version();
    hadron_kernel::kinfo!("AHCI: version {}.{}", major, minor);

    // Bind IRQ line for async completion.
    let _irq = hadron_kernel::drivers::irq::IrqLine::bind_isa(info.interrupt_line, &ctx.irq)
        .map_err(|_| DriverError::InitFailed)?;

    // Unmask the IRQ.
    ctx.irq
        .unmask_irq(info.interrupt_line)
        .map_err(|_| DriverError::InitFailed)?;

    // Enumerate ports.
    let pi = hba.ports_implemented();
    let mut disks = Vec::new();

    for port_num in 0..32u8 {
        if pi & (1 << port_num) == 0 {
            continue;
        }

        hadron_kernel::kdebug!("AHCI: checking port {}", port_num);

        if let Some(port) = AhciPort::init(&hba, port_num, &ctx.dma) {
            if port.identity.is_some() {
                hadron_kernel::kinfo!("AHCI: port {} has device", port_num);

                // Clone the IRQ binding for each disk.
                // All ports on the same HBA share the same IRQ.
                let disk_irq = hadron_kernel::drivers::irq::IrqLine::bind_isa(info.interrupt_line, &ctx.irq)
                    .unwrap_or_else(|_| {
                        // If we can't bind a second time (already registered), reuse
                        // by creating a new IrqLine that references the same vector.
                        hadron_kernel::drivers::irq::IrqLine::from_vector(
                            ctx.irq.isa_irq_vector(info.interrupt_line),
                        )
                    });

                disks.push(AhciDisk {
                    port,
                    irq: disk_irq,
                    dma: ctx.dma,
                });
            }
        }
    }

    if disks.is_empty() {
        hadron_kernel::kwarn!("AHCI: no devices found on any port");
    } else {
        hadron_kernel::kinfo!("AHCI: {} disk(s) discovered", disks.len());
    }

    // Register each disk via DeviceSet.
    let mut devices = DeviceSet::new();
    for disk in disks {
        let idx = {
            let mut counter = DISK_INDEX.lock();
            let i = *counter;
            *counter += 1;
            i
        };
        let path = DevicePath::pci(
            info.address.bus,
            info.address.device,
            info.address.function,
            "ahci",
            idx,
        );
        hadron_kernel::kinfo!("AHCI: registered as \"ahci-{}\"", idx);
        devices.add_block_device(path, disk);
    }

    Ok(PciDriverRegistration {
        devices,
        lifecycle: None,
    })
}
