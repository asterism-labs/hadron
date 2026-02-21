//! VirtIO block device driver (virtio-blk).
//!
//! Implements [`BlockDevice`] for VirtIO block devices discovered via PCI.
//! Supports both MSI-X and legacy INTx interrupt delivery.

extern crate alloc;

use core::ptr;

use hadron_kernel::sync::SpinLock;
use hadron_kernel::driver_api::block::{BlockDevice, IoError};
use hadron_kernel::driver_api::capability::DmaCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::pci::PciDeviceId;

use super::pci::VirtioPciTransport;
use super::queue::{Virtqueue, VIRTQ_DESC_F_WRITE};
use super::{VirtioDevice, VIRTIO_MSI_NO_VECTOR};
use hadron_kernel::drivers::irq::IrqLine;
use crate::pci::msix::MsixTable;

// ---------------------------------------------------------------------------
// PCI IDs
// ---------------------------------------------------------------------------

/// VirtIO vendor ID.
const VIRTIO_VENDOR: u16 = 0x1AF4;
/// VirtIO block device (modern PCI).
const VIRTIO_BLK_DEVICE_MODERN: u16 = 0x1042;
/// VirtIO block device (transitional PCI).
const VIRTIO_BLK_DEVICE_TRANSITIONAL: u16 = 0x1001;

// ---------------------------------------------------------------------------
// VirtIO block request types
// ---------------------------------------------------------------------------

/// VirtIO block request: read.
const VIRTIO_BLK_T_IN: u32 = 0;
/// VirtIO block request: write.
const VIRTIO_BLK_T_OUT: u32 = 1;

/// VirtIO block request status: success.
const VIRTIO_BLK_S_OK: u8 = 0;

/// VirtIO block request header.
#[repr(C)]
struct VirtioBlkReqHeader {
    /// Request type (0 = read, 1 = write).
    type_: u32,
    /// Reserved.
    reserved: u32,
    /// Starting sector.
    sector: u64,
}

/// Page size for DMA allocations.
const PAGE_SIZE: u64 = 4096;

/// Number of descriptors in a block request chain: header + data + status.
const REQ_CHAIN_LEN: usize = 3;

// ---------------------------------------------------------------------------
// VirtioBlkDisk — BlockDevice implementation
// ---------------------------------------------------------------------------

/// A VirtIO block device implementing async block I/O.
pub struct VirtioBlkDisk {
    /// The underlying VirtIO device.
    device: VirtioDevice,
    /// The request virtqueue (index 0).
    queue: SpinLock<Virtqueue>,
    /// IRQ line for async completion notification.
    irq: IrqLine,
    /// DMA capability for memory allocation.
    dma: DmaCapability,
    /// Total number of sectors.
    capacity: u64,
    /// Sector size in bytes.
    sector_size: u32,
}

// SAFETY: VirtioBlkDisk is Send+Sync because all mutable state is behind
// SpinLock, IrqLine is just a u8, and DmaCapability is Copy.
unsafe impl Send for VirtioBlkDisk {}
unsafe impl Sync for VirtioBlkDisk {}

impl BlockDevice for VirtioBlkDisk {
    async fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), IoError> {
        if sector >= self.capacity {
            return Err(IoError::OutOfRange);
        }
        let ss = self.sector_size as usize;
        if buf.len() < ss {
            return Err(IoError::InvalidBuffer);
        }

        // Allocate DMA bounce buffer (1 page covers header + data + status).
        let dma_phys = self
            .dma
            .alloc_frames(1)
            .map_err(|_| IoError::DmaError)?;
        let dma_virt = self.dma.phys_to_virt(dma_phys);

        // Layout within the DMA page:
        //   [0..16)           = VirtioBlkReqHeader
        //   [16..16+ss)       = data buffer
        //   [16+ss..16+ss+1)  = status byte
        let header_phys = dma_phys;
        let data_phys = dma_phys + 16;
        let status_phys = dma_phys + 16 + ss as u64;

        // Write the request header.
        // SAFETY: dma_virt points to a freshly allocated page.
        unsafe {
            let header = &mut *(dma_virt as *mut VirtioBlkReqHeader);
            header.type_ = VIRTIO_BLK_T_IN;
            header.reserved = 0;
            header.sector = sector;

            // Zero the status byte.
            ptr::write_volatile((dma_virt + 16 + ss as u64) as *mut u8, 0xFF);
        }

        // Build the 3-descriptor chain.
        let chain: [(u64, u32, u16); REQ_CHAIN_LEN] = [
            (header_phys, 16, 0),                         // header: device-readable
            (data_phys, ss as u32, VIRTQ_DESC_F_WRITE),   // data: device-writable
            (status_phys, 1, VIRTQ_DESC_F_WRITE),         // status: device-writable
        ];

        {
            let mut vq = self.queue.lock();
            vq.add_buf(&chain).map_err(|_| IoError::DmaError)?;
            vq.notify(self.device.transport(), 0);
        }

        // Wait for completion.
        loop {
            self.irq.wait().await;

            // Read ISR to acknowledge the interrupt.
            self.device.transport().isr_status();

            let mut vq = self.queue.lock();
            if vq.poll_used().is_some() {
                break;
            }
        }

        // Check status byte.
        // SAFETY: The device has completed the request and written the status.
        let status = unsafe { ptr::read_volatile((dma_virt + 16 + ss as u64) as *const u8) };

        let result = if status == VIRTIO_BLK_S_OK {
            // Copy data from DMA buffer to caller's buffer.
            // SAFETY: dma_virt + 16 contains the read data.
            unsafe {
                ptr::copy_nonoverlapping((dma_virt + 16) as *const u8, buf.as_mut_ptr(), ss);
            }
            Ok(())
        } else {
            Err(IoError::DeviceError)
        };

        // Free the DMA bounce buffer.
        // SAFETY: We are done with the DMA buffer.
        unsafe { self.dma.free_frames(dma_phys, 1) };

        result
    }

    async fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), IoError> {
        if sector >= self.capacity {
            return Err(IoError::OutOfRange);
        }
        let ss = self.sector_size as usize;
        if buf.len() < ss {
            return Err(IoError::InvalidBuffer);
        }

        // Allocate DMA bounce buffer.
        let dma_phys = self
            .dma
            .alloc_frames(1)
            .map_err(|_| IoError::DmaError)?;
        let dma_virt = self.dma.phys_to_virt(dma_phys);

        let header_phys = dma_phys;
        let data_phys = dma_phys + 16;
        let status_phys = dma_phys + 16 + ss as u64;

        // Write the request header and copy data.
        // SAFETY: dma_virt points to a freshly allocated page.
        unsafe {
            let header = &mut *(dma_virt as *mut VirtioBlkReqHeader);
            header.type_ = VIRTIO_BLK_T_OUT;
            header.reserved = 0;
            header.sector = sector;

            // Copy the write data.
            ptr::copy_nonoverlapping(buf.as_ptr(), (dma_virt + 16) as *mut u8, ss);

            // Initialize status byte.
            ptr::write_volatile((dma_virt + 16 + ss as u64) as *mut u8, 0xFF);
        }

        // Build the 3-descriptor chain.
        let chain: [(u64, u32, u16); REQ_CHAIN_LEN] = [
            (header_phys, 16, 0),                         // header: device-readable
            (data_phys, ss as u32, 0),                     // data: device-readable
            (status_phys, 1, VIRTQ_DESC_F_WRITE),         // status: device-writable
        ];

        {
            let mut vq = self.queue.lock();
            vq.add_buf(&chain).map_err(|_| IoError::DmaError)?;
            vq.notify(self.device.transport(), 0);
        }

        // Wait for completion.
        loop {
            self.irq.wait().await;
            self.device.transport().isr_status();

            let mut vq = self.queue.lock();
            if vq.poll_used().is_some() {
                break;
            }
        }

        // Check status byte.
        // SAFETY: The device has completed the request.
        let status = unsafe { ptr::read_volatile((dma_virt + 16 + ss as u64) as *const u8) };

        // Free the DMA bounce buffer.
        // SAFETY: We are done with the DMA buffer.
        unsafe { self.dma.free_frames(dma_phys, 1) };

        if status == VIRTIO_BLK_S_OK {
            Ok(())
        } else {
            Err(IoError::DeviceError)
        }
    }

    fn sector_size(&self) -> usize {
        self.sector_size as usize
    }

    fn sector_count(&self) -> u64 {
        self.capacity
    }
}

/// Counter for assigning unique device names to discovered disks.
static DISK_INDEX: SpinLock<usize> = SpinLock::named("VIRTIO_DISK_INDEX", 0);

// ---------------------------------------------------------------------------
// PCI registration
// ---------------------------------------------------------------------------

/// PCI device ID table for VirtIO block devices.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 2] = [
    PciDeviceId::new(VIRTIO_VENDOR, VIRTIO_BLK_DEVICE_MODERN),
    PciDeviceId::new(VIRTIO_VENDOR, VIRTIO_BLK_DEVICE_TRANSITIONAL),
];

/// VirtIO block driver registration type.
struct VirtioBlkDriver;

#[hadron_driver_macros::hadron_driver(
    name = "virtio-blk",
    kind = pci,
    capabilities = [Irq, Mmio, Dma, PciConfig],
    pci_ids = &ID_TABLE,
)]
impl VirtioBlkDriver {
    /// PCI probe function for VirtIO block devices.
    fn probe(
        ctx: DriverContext,
    ) -> Result<hadron_kernel::driver_api::registration::PciDriverRegistration, DriverError> {
        use hadron_kernel::driver_api::capability::{
            CapabilityAccess, DmaCapability, IrqCapability, MmioCapability, PciConfigCapability,
        };
        use hadron_kernel::driver_api::device_path::DevicePath;
        use hadron_kernel::driver_api::registration::{DeviceSet, PciDriverRegistration};

        let info = ctx.device();
        let pci_config = ctx.capability::<PciConfigCapability>();
        let mmio_cap = ctx.capability::<MmioCapability>();
        let irq_cap = ctx.capability::<IrqCapability>();
        let dma = ctx.capability::<DmaCapability>();

        hadron_kernel::kinfo!(
            "virtio-blk: probing {:04x}:{:04x} at {}",
            info.vendor_id,
            info.device_id,
            info.address
        );

        // Enable bus mastering.
        pci_config.enable_bus_mastering();

        // Initialize VirtIO PCI transport.
        let transport = VirtioPciTransport::new(info, mmio_cap)?;

        // Try MSI-X setup, fall back to legacy.
        let (irq, msix_table) = setup_irq(info, &transport, irq_cap, mmio_cap)?;

        // Initialize VirtIO device (steps 1-6).
        // No device-specific feature bits needed for basic block I/O.
        let device = VirtioDevice::init(transport, 0)?;

        // Read device config.
        let capacity = device
            .transport()
            .device_cfg_read_u64(0)
            .unwrap_or(0);

        // blk_size is at device config offset 20 (u32).
        let blk_size = device
            .transport()
            .device_cfg_read_u32(20)
            .unwrap_or(512);

        hadron_kernel::kinfo!(
            "virtio-blk: capacity={} sectors, sector_size={}",
            capacity,
            blk_size
        );

        // Setup request queue (queue 0).
        let vq = device.setup_queue(0, dma)?;

        // If using MSI-X, configure the queue's MSI-X vector.
        if let Some(ref msix) = msix_table {
            device.transport().set_queue_select(0);
            device.transport().set_queue_msix_vector(0);
            let readback = device.transport().queue_msix_vector();
            if readback == VIRTIO_MSI_NO_VECTOR {
                hadron_kernel::kwarn!("virtio-blk: failed to set queue MSI-X vector");
            }
            // Unmask and enable MSI-X.
            msix.unmask(0);
            msix.enable();
        }

        // Complete initialization.
        device.set_driver_ok();

        hadron_kernel::kinfo!("virtio-blk: device ready, irq vector {}", irq.vector());

        let disk = VirtioBlkDisk {
            device,
            queue: SpinLock::new(vq),
            irq,
            dma: *dma,
            capacity,
            sector_size: blk_size,
        };

        // Register in the kernel device registry via DeviceSet.
        let idx = {
            let mut counter = DISK_INDEX.lock();
            let i = *counter;
            *counter += 1;
            i
        };

        let mut devices = DeviceSet::new();
        let path = DevicePath::pci(
            info.address.bus,
            info.address.device,
            info.address.function,
            "virtio-blk",
            idx,
        );
        devices.add_block_device(path, disk);

        hadron_kernel::kinfo!("virtio-blk: registered as \"virtio-blk-{}\"", idx);
        Ok(PciDriverRegistration {
            devices,
            lifecycle: None,
        })
    }
}

/// Sets up IRQ delivery (MSI-X preferred, legacy fallback).
#[cfg(target_os = "none")]
fn setup_irq(
    info: &hadron_kernel::driver_api::pci::PciDeviceInfo,
    transport: &VirtioPciTransport,
    irq_cap: &hadron_kernel::driver_api::capability::IrqCapability,
    mmio_cap: &hadron_kernel::driver_api::capability::MmioCapability,
) -> Result<(IrqLine, Option<MsixTable>), DriverError> {
    if let Some(msix_cap) = transport.msix_cap() {
        // Try MSI-X.
        match try_setup_msix(info, msix_cap, irq_cap, mmio_cap) {
            Ok((irq, table)) => {
                hadron_kernel::kinfo!(
                    "virtio-blk: MSI-X enabled, vector {}",
                    irq.vector()
                );
                return Ok((irq, Some(table)));
            }
            Err(e) => {
                hadron_kernel::kwarn!("virtio-blk: MSI-X setup failed ({:?}), falling back to legacy", e);
            }
        }
    }

    // Legacy INTx fallback.
    let irq = IrqLine::bind_isa(info.interrupt_line, irq_cap)
        .map_err(|_| DriverError::InitFailed)?;
    irq_cap
        .unmask_irq(info.interrupt_line)
        .map_err(|_| DriverError::InitFailed)?;

    Ok((irq, None))
}

/// Attempts to set up MSI-X for the device.
#[cfg(target_os = "none")]
fn try_setup_msix(
    info: &hadron_kernel::driver_api::pci::PciDeviceInfo,
    msix_cap: &crate::pci::caps::MsixCapability,
    irq_cap: &hadron_kernel::driver_api::capability::IrqCapability,
    mmio_cap: &hadron_kernel::driver_api::capability::MmioCapability,
) -> Result<(IrqLine, MsixTable), DriverError> {
    let msix_table = MsixTable::setup(info, msix_cap, mmio_cap)?;

    // Allocate a vector for the request queue.
    let vector = irq_cap.alloc_vector()?;

    // Bind the IRQ handler.
    let irq = IrqLine::bind(vector, irq_cap)?;

    // Configure MSI-X entry 0 for CPU 0.
    msix_table.set_entry(0, vector, 0);

    Ok((irq, msix_table))
}
