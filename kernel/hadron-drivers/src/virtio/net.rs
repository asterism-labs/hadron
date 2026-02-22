//! VirtIO network device driver (virtio-net).
//!
//! Implements [`NetworkDevice`] for VirtIO network devices discovered via PCI.
//! Supports both MSI-X and legacy INTx interrupt delivery.

extern crate alloc;

use core::ptr;

use hadron_kernel::driver_api::capability::DmaCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::net::{MacAddress, NetError, NetworkDevice};
use hadron_kernel::driver_api::pci::PciDeviceId;
use hadron_kernel::sync::SpinLock;

use super::pci::VirtioPciTransport;
use super::queue::{VIRTQ_DESC_F_WRITE, Virtqueue};
use super::{VIRTIO_MSI_NO_VECTOR, VirtioDevice};
use crate::pci::msix::MsixTable;
use hadron_kernel::drivers::irq::IrqLine;

// ---------------------------------------------------------------------------
// PCI IDs
// ---------------------------------------------------------------------------

/// VirtIO vendor ID.
const VIRTIO_VENDOR: u16 = 0x1AF4;
/// VirtIO network device (modern PCI).
const VIRTIO_NET_DEVICE_MODERN: u16 = 0x1041;
/// VirtIO network device (transitional PCI).
const VIRTIO_NET_DEVICE_TRANSITIONAL: u16 = 0x1000;

// ---------------------------------------------------------------------------
// VirtIO-net feature bits (low 32 bits of feature dword 0)
// ---------------------------------------------------------------------------

/// Device has a valid MAC address in device config.
const VIRTIO_NET_F_MAC: u32 = 1 << 5;
/// Device reports link status via `status` field.
const VIRTIO_NET_F_STATUS: u32 = 1 << 16;
/// Device has a valid MTU field in device config.
const VIRTIO_NET_F_MTU: u32 = 1 << 3;

/// Feature mask: accept MAC, STATUS, and MTU; reject CSUM/GSO/MRG_RXBUF.
const DESIRED_FEATURES: u32 = VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS | VIRTIO_NET_F_MTU;

// ---------------------------------------------------------------------------
// VirtIO-net header (10 bytes, no MRG_RXBUF)
// ---------------------------------------------------------------------------

/// VirtIO network header prepended to every packet.
///
/// Without VIRTIO_NET_F_MRG_RXBUF, this is exactly 10 bytes.
#[repr(C)]
struct VirtioNetHdr {
    flags: u8,
    gso_type: u8,
    hdr_len: u16,
    gso_size: u16,
    csum_start: u16,
    csum_offset: u16,
}

/// Size of VirtioNetHdr in bytes.
const NET_HDR_SIZE: usize = 10;

/// Page size for DMA allocations.
const PAGE_SIZE: u64 = 4096;

/// Number of pre-allocated RX buffers.
const RX_POOL_SIZE: usize = 64;

/// Default MTU: 14-byte Ethernet header + 1500-byte payload.
const DEFAULT_MTU: usize = 1514;

// ---------------------------------------------------------------------------
// RX buffer pool
// ---------------------------------------------------------------------------

/// A pool of pre-allocated DMA pages for receiving packets.
struct RxBufferPool {
    /// Physical addresses of each buffer page.
    bufs: [u64; RX_POOL_SIZE],
    /// DMA capability for phys→virt translation and deallocation.
    dma: DmaCapability,
}

impl RxBufferPool {
    /// Allocates the RX buffer pool.
    fn new(dma: &DmaCapability) -> Result<Self, DriverError> {
        let mut bufs = [0u64; RX_POOL_SIZE];
        for buf in bufs.iter_mut() {
            *buf = dma.alloc_frames(1)?;
        }
        Ok(Self { bufs, dma: *dma })
    }

    /// Returns (phys, virt) for the buffer at the given index.
    fn get(&self, idx: usize) -> (u64, u64) {
        let phys = self.bufs[idx];
        let virt = self.dma.phys_to_virt(phys);
        (phys, virt)
    }
}

// ---------------------------------------------------------------------------
// RX queue state (under lock)
// ---------------------------------------------------------------------------

/// State associated with the RX virtqueue, protected by a SpinLock.
struct RxQueueState {
    /// The RX virtqueue.
    queue: Virtqueue,
    /// Maps descriptor index → RX pool buffer index.
    desc_to_buf: [u8; 256],
}

// ---------------------------------------------------------------------------
// VirtioNetNic — NetworkDevice implementation
// ---------------------------------------------------------------------------

/// A VirtIO network device implementing async Ethernet frame I/O.
pub struct VirtioNetNic {
    /// The underlying VirtIO device.
    device: VirtioDevice,
    /// RX queue state (virtqueue + descriptor-to-buffer mapping).
    rx: SpinLock<RxQueueState>,
    /// TX virtqueue.
    tx_queue: SpinLock<Virtqueue>,
    /// IRQ line for async completion notification.
    irq: IrqLine,
    /// DMA capability for memory allocation.
    dma: DmaCapability,
    /// Pre-allocated RX buffer pool.
    rx_pool: RxBufferPool,
    /// Device MAC address.
    mac: MacAddress,
    /// Maximum transmission unit (Ethernet header + payload).
    mtu: usize,
}

// SAFETY: VirtioNetNic is Send+Sync because all mutable state is behind
// SpinLock, IrqLine is just a wrapper around a vector, and DmaCapability is Copy.
unsafe impl Send for VirtioNetNic {}
unsafe impl Sync for VirtioNetNic {}

impl NetworkDevice for VirtioNetNic {
    async fn recv(&self, buf: &mut [u8]) -> Result<usize, NetError> {
        loop {
            // Wait for an interrupt (RX completion).
            self.irq.wait().await;

            // Acknowledge the interrupt.
            self.device.transport().isr_status();

            let mut rx = self.rx.lock();
            if let Some((head, written)) = rx.queue.poll_used() {
                // Determine which pool buffer was used.
                let pool_idx = rx.desc_to_buf[head as usize] as usize;
                let (_phys, virt) = self.rx_pool.get(pool_idx);

                // The device wrote: VirtioNetHdr (10 bytes) + Ethernet frame.
                let total = written as usize;
                if total <= NET_HDR_SIZE {
                    // Empty or header-only packet, repost and continue.
                    self.repost_rx_buffer(&mut rx, pool_idx);
                    continue;
                }

                let pkt_len = total - NET_HDR_SIZE;
                if buf.len() < pkt_len {
                    // Repost the buffer even on error so we don't leak it.
                    self.repost_rx_buffer(&mut rx, pool_idx);
                    return Err(NetError::BufferTooSmall);
                }

                // Copy the Ethernet frame (skip the VirtIO header).
                // SAFETY: virt points to a valid DMA page, and the device wrote `total` bytes.
                unsafe {
                    ptr::copy_nonoverlapping(
                        (virt + NET_HDR_SIZE as u64) as *const u8,
                        buf.as_mut_ptr(),
                        pkt_len,
                    );
                }

                // Repost the buffer for future receives.
                self.repost_rx_buffer(&mut rx, pool_idx);

                return Ok(pkt_len);
            }
            // Spurious wakeup (e.g., TX completion on shared vector), loop again.
        }
    }

    async fn send(&self, buf: &[u8]) -> Result<(), NetError> {
        if buf.len() > self.mtu {
            return Err(NetError::PacketTooLarge);
        }

        let total_len = NET_HDR_SIZE + buf.len();

        // Allocate a DMA page for the TX buffer.
        let dma_phys = self.dma.alloc_frames(1).map_err(|_| NetError::DmaError)?;
        let dma_virt = self.dma.phys_to_virt(dma_phys);

        // Write a zeroed VirtIO-net header followed by the packet data.
        // SAFETY: dma_virt points to a freshly allocated page (4096 bytes).
        unsafe {
            ptr::write_bytes(dma_virt as *mut u8, 0, NET_HDR_SIZE);
            ptr::copy_nonoverlapping(
                buf.as_ptr(),
                (dma_virt + NET_HDR_SIZE as u64) as *mut u8,
                buf.len(),
            );
        }

        // Post as a single device-readable descriptor.
        let chain: [(u64, u32, u16); 1] = [(dma_phys, total_len as u32, 0)];

        {
            let mut vq = self.tx_queue.lock();
            vq.add_buf(&chain).map_err(|_| NetError::TxQueueFull)?;
            vq.notify(self.device.transport(), 1);
        }

        // Wait for TX completion.
        loop {
            self.irq.wait().await;
            self.device.transport().isr_status();

            let mut vq = self.tx_queue.lock();
            if vq.poll_used().is_some() {
                break;
            }
        }

        // Free the TX DMA buffer.
        // SAFETY: We are done with the DMA buffer.
        unsafe { self.dma.free_frames(dma_phys, 1) };

        Ok(())
    }

    fn mac_address(&self) -> MacAddress {
        self.mac
    }

    fn mtu(&self) -> usize {
        self.mtu
    }
}

impl VirtioNetNic {
    /// Reposts an RX pool buffer back to the RX virtqueue for future receives.
    fn repost_rx_buffer(&self, rx: &mut RxQueueState, pool_idx: usize) {
        let (phys, _virt) = self.rx_pool.get(pool_idx);
        let buf_size = (NET_HDR_SIZE + self.mtu) as u32;
        let chain: [(u64, u32, u16); 1] = [(phys, buf_size, VIRTQ_DESC_F_WRITE)];

        if let Ok(head) = rx.queue.add_buf(&chain) {
            rx.desc_to_buf[head as usize] = pool_idx as u8;
            rx.queue.notify(self.device.transport(), 0);
        }
    }
}

/// Counter for assigning unique device names to discovered NICs.
static NIC_INDEX: SpinLock<usize> = SpinLock::leveled("VIRTIO_NIC_INDEX", 6, 0);

// ---------------------------------------------------------------------------
// PCI registration
// ---------------------------------------------------------------------------

/// PCI device ID table for VirtIO network devices.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 2] = [
    PciDeviceId::new(VIRTIO_VENDOR, VIRTIO_NET_DEVICE_MODERN),
    PciDeviceId::new(VIRTIO_VENDOR, VIRTIO_NET_DEVICE_TRANSITIONAL),
];

/// VirtIO network driver registration type.
struct VirtioNetDriver;

#[hadron_driver_macros::hadron_driver(
    name = "virtio-net",
    kind = pci,
    capabilities = [Irq, Mmio, Dma, PciConfig],
    pci_ids = &ID_TABLE,
)]
impl VirtioNetDriver {
    /// PCI probe function for VirtIO network devices.
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
            "virtio-net: probing {:04x}:{:04x} at {}",
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
        let device = VirtioDevice::init(transport, DESIRED_FEATURES)?;

        // Read MAC address from device config (6 bytes at offsets 0-5).
        let mut mac_bytes = [0u8; 6];
        for (i, byte) in mac_bytes.iter_mut().enumerate() {
            *byte = device.transport().device_cfg_read_u8(i as u32).unwrap_or(0);
        }
        let mac = MacAddress(mac_bytes);

        // Read MTU from device config (offset 10, u16), fall back to default.
        let mtu = device
            .transport()
            .device_cfg_read_u16(10)
            .map(|raw_mtu| {
                // Device reports IP MTU; add 14 bytes for Ethernet header.
                (raw_mtu as usize) + 14
            })
            .unwrap_or(DEFAULT_MTU);

        hadron_kernel::kinfo!("virtio-net: MAC={}, MTU={}", mac, mtu);

        // Setup RX queue (queue 0).
        let rx_vq = device.setup_queue(0, dma)?;

        // Setup TX queue (queue 1).
        let tx_vq = device.setup_queue(1, dma)?;

        // If using MSI-X, configure both queues to use shared vector 0.
        if let Some(ref msix) = msix_table {
            // RX queue (index 0) → MSI-X vector 0.
            device.transport().set_queue_select(0);
            device.transport().set_queue_msix_vector(0);
            let readback = device.transport().queue_msix_vector();
            if readback == VIRTIO_MSI_NO_VECTOR {
                hadron_kernel::kwarn!("virtio-net: failed to set RX queue MSI-X vector");
            }

            // TX queue (index 1) → MSI-X vector 0 (shared).
            device.transport().set_queue_select(1);
            device.transport().set_queue_msix_vector(0);
            let readback = device.transport().queue_msix_vector();
            if readback == VIRTIO_MSI_NO_VECTOR {
                hadron_kernel::kwarn!("virtio-net: failed to set TX queue MSI-X vector");
            }

            // Unmask and enable MSI-X.
            msix.unmask(0);
            msix.enable();
        }

        // Complete initialization.
        device.set_driver_ok();

        hadron_kernel::kinfo!("virtio-net: device ready, irq vector {}", irq.vector());

        // Allocate RX buffer pool.
        let rx_pool = RxBufferPool::new(dma).map_err(|_| DriverError::InitFailed)?;

        // Pre-post all RX buffers.
        let buf_size = (NET_HDR_SIZE + mtu) as u32;
        let mut desc_to_buf = [0u8; 256];
        let mut rx_vq = rx_vq; // make mutable for add_buf
        for i in 0..RX_POOL_SIZE {
            let (phys, _virt) = rx_pool.get(i);
            let chain: [(u64, u32, u16); 1] = [(phys, buf_size, VIRTQ_DESC_F_WRITE)];
            let head = rx_vq.add_buf(&chain).map_err(|_| DriverError::InitFailed)?;
            desc_to_buf[head as usize] = i as u8;
        }
        // Kick the device to start processing RX buffers.
        rx_vq.notify(device.transport(), 0);

        let nic = VirtioNetNic {
            device,
            rx: SpinLock::named(
                "VirtioNet.rx",
                RxQueueState {
                    queue: rx_vq,
                    desc_to_buf,
                },
            ),
            tx_queue: SpinLock::named("VirtioNet.tx", tx_vq),
            irq,
            dma: *dma,
            rx_pool,
            mac,
            mtu,
        };

        // Register in the kernel device registry via DeviceSet.
        let idx = {
            let mut counter = NIC_INDEX.lock();
            let i = *counter;
            *counter += 1;
            i
        };

        let mut devices = DeviceSet::new();
        let path = DevicePath::pci(
            info.address.bus,
            info.address.device,
            info.address.function,
            "virtio-net",
            idx,
        );
        devices.add_net_device(path, nic);

        hadron_kernel::kinfo!("virtio-net: registered as \"virtio-net-{}\"", idx);
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
                hadron_kernel::kinfo!("virtio-net: MSI-X enabled, vector {}", irq.vector());
                return Ok((irq, Some(table)));
            }
            Err(e) => {
                hadron_kernel::kwarn!(
                    "virtio-net: MSI-X setup failed ({:?}), falling back to legacy",
                    e
                );
            }
        }
    }

    // Legacy INTx fallback.
    let irq =
        IrqLine::bind_isa(info.interrupt_line, irq_cap).map_err(|_| DriverError::InitFailed)?;
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

    // Allocate a vector for RX/TX (shared).
    let vector = irq_cap.alloc_vector()?;

    // Bind the IRQ handler.
    let irq = IrqLine::bind(vector, irq_cap)?;

    // Configure MSI-X entry 0 for CPU 0.
    msix_table.set_entry(0, vector.as_irq_vector(), 0);

    Ok((irq, msix_table))
}
