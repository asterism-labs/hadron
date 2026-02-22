//! Intel e1000e (82574L) Ethernet controller driver.
//!
//! Implements [`NetworkDevice`] for the Intel 82574L NIC discovered via PCI.
//! Uses legacy 16-byte descriptors with direct MMIO register access. Supports
//! MSI-X with legacy INTx fallback.

extern crate alloc;

use core::ptr;

use hadron_kernel::driver_api::capability::DmaCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::net::{MacAddress, NetError, NetworkDevice};
use hadron_kernel::driver_api::pci::{PciBar, PciDeviceId};
use hadron_kernel::drivers::irq::IrqLine;
use hadron_kernel::sync::SpinLock;

use crate::pci::msix::MsixTable;

pub mod regs;

use regs::{
    Ctrl, E1000eRegs, ICR_LSC, ICR_RXDMT, ICR_RXT0, ICR_TXDW, MTA_BASE, MTA_COUNT, RXD_STAT_DD,
    Rctl, RxDesc, TXD_CMD_EOP, TXD_CMD_IFCS, TXD_CMD_RS, TXD_STAT_DD, Tctl, TxDesc,
};

// ---------------------------------------------------------------------------
// PCI IDs
// ---------------------------------------------------------------------------

/// Intel vendor ID.
const INTEL_VENDOR: u16 = 0x8086;
/// Intel 82574L device ID.
const E1000E_82574L: u16 = 0x10D3;

// ---------------------------------------------------------------------------
// Ring / buffer constants
// ---------------------------------------------------------------------------

/// Number of RX descriptors.
const RX_RING_SIZE: usize = 64;
/// Number of TX descriptors.
const TX_RING_SIZE: usize = 64;
/// Size of each RX buffer (matches RCTL BSIZE=00 → 2048 bytes).
const RX_BUF_SIZE: usize = 2048;
/// Page size for DMA allocations.
const PAGE_SIZE: u64 = 4096;
/// Maximum Ethernet frame size (14-byte header + 1500-byte payload).
const MAX_FRAME_SIZE: usize = 1514;

// ---------------------------------------------------------------------------
// RX / TX ring state (under lock)
// ---------------------------------------------------------------------------

/// Mutable state for the RX descriptor ring.
struct RxRingState {
    /// Virtual address of the RX descriptor ring.
    ring_virt: *mut RxDesc,
    /// Software tail pointer (next descriptor to consume).
    tail: u16,
}

// SAFETY: The ring pointer is DMA memory managed by the driver; access is
// always gated by the enclosing SpinLock.
unsafe impl Send for RxRingState {}

/// Mutable state for the TX descriptor ring.
struct TxRingState {
    /// Virtual address of the TX descriptor ring.
    ring_virt: *mut TxDesc,
    /// Software tail pointer (next descriptor to post).
    tail: u16,
}

// SAFETY: Same as RxRingState.
unsafe impl Send for TxRingState {}

// ---------------------------------------------------------------------------
// E1000eNic — NetworkDevice implementation
// ---------------------------------------------------------------------------

/// An Intel e1000e network device implementing async Ethernet frame I/O.
pub struct E1000eNic {
    /// MMIO register block.
    regs: E1000eRegs,
    /// RX ring state.
    rx: SpinLock<RxRingState>,
    /// TX ring state.
    tx: SpinLock<TxRingState>,
    /// Physical addresses of each RX buffer page.
    rx_bufs: [u64; RX_RING_SIZE],
    /// IRQ line for async completion notification.
    irq: IrqLine,
    /// DMA capability for memory allocation.
    dma: DmaCapability,
    /// Device MAC address.
    mac: MacAddress,
}

// SAFETY: E1000eNic is Send+Sync because all mutable state is behind
// SpinLock, IrqLine is just a vector wrapper, and DmaCapability is Copy.
unsafe impl Send for E1000eNic {}
unsafe impl Sync for E1000eNic {}

impl NetworkDevice for E1000eNic {
    async fn recv(&self, buf: &mut [u8]) -> Result<usize, NetError> {
        loop {
            self.irq.wait().await;

            // Read ICR to acknowledge + get cause bits.
            let _icr = self.regs.icr();

            let mut rx = self.rx.lock();
            let next = ((rx.tail as usize + 1) % RX_RING_SIZE) as u16;

            // SAFETY: next is in bounds (0..RX_RING_SIZE) and ring_virt
            // points to a valid DMA descriptor ring.
            let desc = unsafe { &mut *rx.ring_virt.add(next as usize) };

            if desc.status & RXD_STAT_DD == 0 {
                // Spurious wakeup (e.g. TX completion), loop again.
                continue;
            }

            if desc.errors != 0 {
                // Error frame: reset descriptor and advance.
                desc.status = 0;
                rx.tail = next;
                self.regs.set_rdt(u32::from(next));
                continue;
            }

            let pkt_len = desc.length as usize;
            if buf.len() < pkt_len {
                // Repost descriptor even on error so we don't leak it.
                desc.status = 0;
                rx.tail = next;
                self.regs.set_rdt(u32::from(next));
                return Err(NetError::BufferTooSmall);
            }

            // Copy frame data from the RX buffer.
            let rx_virt = self.dma.phys_to_virt(self.rx_bufs[next as usize]);
            // SAFETY: rx_virt points to a valid DMA page and the hardware
            // wrote pkt_len bytes into it.
            unsafe {
                ptr::copy_nonoverlapping(rx_virt as *const u8, buf.as_mut_ptr(), pkt_len);
            }

            // Give the descriptor back to hardware.
            desc.status = 0;
            rx.tail = next;
            self.regs.set_rdt(u32::from(next));

            return Ok(pkt_len);
        }
    }

    async fn send(&self, buf: &[u8]) -> Result<(), NetError> {
        if buf.len() > MAX_FRAME_SIZE {
            return Err(NetError::PacketTooLarge);
        }

        // Allocate a DMA page for the packet data.
        let dma_phys = self.dma.alloc_frames(1).map_err(|_| NetError::DmaError)?;
        let dma_virt = self.dma.phys_to_virt(dma_phys);

        // Copy the frame into the DMA buffer.
        // SAFETY: dma_virt points to a freshly allocated 4096-byte page.
        unsafe {
            ptr::copy_nonoverlapping(buf.as_ptr(), dma_virt as *mut u8, buf.len());
        }

        let idx;
        {
            let mut tx = self.tx.lock();
            let i = tx.tail as usize;

            // SAFETY: i is in bounds (0..TX_RING_SIZE).
            let desc = unsafe { &mut *tx.ring_virt.add(i) };

            if desc.status & TXD_STAT_DD == 0 {
                // SAFETY: We are done with the DMA buffer.
                unsafe { self.dma.free_frames(dma_phys, 1) };
                return Err(NetError::TxQueueFull);
            }

            desc.addr = dma_phys;
            desc.length = buf.len() as u16;
            desc.cmd = TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS;
            desc.status = 0;
            desc.cso = 0;
            desc.css = 0;
            desc.special = 0;

            idx = i;
            tx.tail = ((i + 1) % TX_RING_SIZE) as u16;

            // Kick hardware.
            self.regs.set_tdt(tx.tail as u32);
        }

        // Wait for TX completion.
        loop {
            self.irq.wait().await;
            let _icr = self.regs.icr();

            // SAFETY: idx is in bounds.
            let desc = unsafe { &*self.tx.lock().ring_virt.add(idx) };
            if desc.status & TXD_STAT_DD != 0 {
                break;
            }
        }

        // Free the TX DMA buffer.
        // SAFETY: Hardware is done with the buffer (DD is set).
        unsafe { self.dma.free_frames(dma_phys, 1) };

        Ok(())
    }

    fn mac_address(&self) -> MacAddress {
        self.mac
    }

    fn mtu(&self) -> usize {
        MAX_FRAME_SIZE
    }
}

// ---------------------------------------------------------------------------
// EEPROM read helper
// ---------------------------------------------------------------------------

/// Reads a 16-bit word from the EEPROM via the EERD register.
fn eeprom_read(regs: &E1000eRegs, addr: u8) -> u16 {
    regs.set_eerd(((addr as u32) << 8) | 0x01);
    loop {
        let val = regs.eerd();
        if val & (1 << 4) != 0 {
            return (val >> 16) as u16;
        }
    }
}

/// Reads the MAC address, trying RAL0/RAH0 first and falling back to EEPROM.
fn read_mac(regs: &E1000eRegs) -> MacAddress {
    let ral = regs.ral0();
    let rah = regs.rah0();

    if ral != 0 || (rah & 0xFFFF) != 0 {
        // RAL0/RAH0 populated by EEPROM auto-load.
        MacAddress([
            (ral & 0xFF) as u8,
            ((ral >> 8) & 0xFF) as u8,
            ((ral >> 16) & 0xFF) as u8,
            ((ral >> 24) & 0xFF) as u8,
            (rah & 0xFF) as u8,
            ((rah >> 8) & 0xFF) as u8,
        ])
    } else {
        // Fallback: read MAC from EEPROM words 0, 1, 2.
        let w0 = eeprom_read(regs, 0);
        let w1 = eeprom_read(regs, 1);
        let w2 = eeprom_read(regs, 2);

        MacAddress([
            (w0 & 0xFF) as u8,
            ((w0 >> 8) & 0xFF) as u8,
            (w1 & 0xFF) as u8,
            ((w1 >> 8) & 0xFF) as u8,
            (w2 & 0xFF) as u8,
            ((w2 >> 8) & 0xFF) as u8,
        ])
    }
}

// ---------------------------------------------------------------------------
// MTA clearing helper
// ---------------------------------------------------------------------------

/// Zeros the 128-entry Multicast Table Array.
fn clear_mta(regs: &E1000eRegs) {
    for i in 0..MTA_COUNT {
        let offset = MTA_BASE + (i as u64) * 4;
        // SAFETY: The register block base covers the full 128 KiB MMIO region;
        // MTA entries span 0x5200..0x5400, well within bounds.
        unsafe {
            core::ptr::write_volatile((regs.base().as_u64() + offset) as *mut u32, 0);
        }
    }
}

// ---------------------------------------------------------------------------
// IRQ setup (MSI-X preferred, legacy INTx fallback)
// ---------------------------------------------------------------------------

/// Sets up IRQ delivery for the e1000e device.
#[cfg(target_os = "none")]
fn setup_irq(
    info: &hadron_kernel::driver_api::pci::PciDeviceInfo,
    irq_cap: &hadron_kernel::driver_api::capability::IrqCapability,
    mmio_cap: &hadron_kernel::driver_api::capability::MmioCapability,
) -> Result<(IrqLine, Option<MsixTable>), DriverError> {
    // Walk PCI capabilities to find MSI-X.
    if let Some(caps) = crate::pci::caps::walk_capabilities(&info.address) {
        for cap in caps {
            if cap.id == crate::pci::cam::regs::CAP_ID_MSIX {
                let msix_cap = crate::pci::caps::read_msix_cap(&info.address, cap.offset);
                match try_setup_msix(info, &msix_cap, irq_cap, mmio_cap) {
                    Ok((irq, table)) => {
                        hadron_kernel::kinfo!("e1000e: MSI-X enabled, vector {}", irq.vector());
                        return Ok((irq, Some(table)));
                    }
                    Err(e) => {
                        hadron_kernel::kwarn!(
                            "e1000e: MSI-X setup failed ({:?}), falling back to legacy",
                            e
                        );
                    }
                }
                break;
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

    // Allocate a shared vector for RX + TX.
    let vector = irq_cap.alloc_vector()?;

    // Bind the IRQ handler.
    let irq = IrqLine::bind(vector, irq_cap)?;

    // Configure MSI-X entry 0 for CPU 0.
    msix_table.set_entry(0, vector.as_irq_vector(), 0);

    Ok((irq, msix_table))
}

// ---------------------------------------------------------------------------
// NIC index counter
// ---------------------------------------------------------------------------

/// Counter for assigning unique device names to discovered e1000e NICs.
static NIC_INDEX: SpinLock<usize> = SpinLock::leveled("E1000E_NIC_INDEX", 6, 0);

// ---------------------------------------------------------------------------
// PCI registration
// ---------------------------------------------------------------------------

/// PCI device ID table for Intel e1000e controllers.
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 1] = [PciDeviceId::new(INTEL_VENDOR, E1000E_82574L)];

/// e1000e driver registration type.
struct E1000eDriver;

#[hadron_driver_macros::hadron_driver(
    name = "e1000e",
    kind = pci,
    capabilities = [Irq, Mmio, Dma, PciConfig],
    pci_ids = &ID_TABLE,
)]
impl E1000eDriver {
    /// PCI probe function for Intel e1000e controllers.
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
            "e1000e: probing {:04x}:{:04x} at {}",
            info.vendor_id,
            info.device_id,
            info.address
        );

        // 1. Enable bus mastering + memory space.
        pci_config.enable_bus_mastering();

        // 2. Map BAR0 as MMIO.
        let (bar_phys, bar_size) = match info.bars[0] {
            PciBar::Memory { base, size, .. } => (base, size),
            _ => {
                hadron_kernel::kwarn!("e1000e: BAR0 is not a memory BAR");
                return Err(DriverError::InitFailed);
            }
        };
        let mmio = mmio_cap.map_mmio(bar_phys, bar_size)?;

        // SAFETY: mmio.virt_base() points to a valid mapped MMIO region.
        let regs = unsafe { E1000eRegs::new(mmio.virt_base()) };

        // 3. Setup IRQ (MSI-X preferred, legacy fallback).
        let (irq, msix_table) = setup_irq(info, irq_cap, mmio_cap)?;

        // 4. Reset device: write CTRL |= RST, spin-wait for self-clear.
        regs.set_ctrl(Ctrl::RST);
        // Spin-wait ~1 ms for reset to complete (RST self-clears).
        for _ in 0..100_000 {
            if !regs.ctrl().contains(Ctrl::RST) {
                break;
            }
            core::hint::spin_loop();
        }

        // 5. Disable interrupts during initialization.
        regs.set_imc(0xFFFF_FFFF);
        let _ = regs.icr(); // clear pending

        // 6. Read MAC address.
        let mac = read_mac(&regs);
        hadron_kernel::kinfo!("e1000e: MAC={}", mac);

        // 7. Set link up.
        regs.set_ctrl(Ctrl::SLU);

        // 8. Clear Multicast Table Array.
        clear_mta(&regs);

        // 9. Init RX ring.
        // Allocate descriptor ring (64 × 16 = 1024 bytes, fits in one page).
        let rx_ring_phys = dma.alloc_frames(1).map_err(|_| DriverError::InitFailed)?;
        let rx_ring_virt = dma.phys_to_virt(rx_ring_phys) as *mut RxDesc;

        // Zero the descriptor ring.
        // SAFETY: Freshly allocated DMA page.
        unsafe { ptr::write_bytes(rx_ring_virt, 0, RX_RING_SIZE) };

        // Allocate per-descriptor RX buffers (one page each).
        let mut rx_bufs = [0u64; RX_RING_SIZE];
        for i in 0..RX_RING_SIZE {
            let buf_phys = dma.alloc_frames(1).map_err(|_| DriverError::InitFailed)?;
            rx_bufs[i] = buf_phys;

            // SAFETY: rx_ring_virt[i] is within the allocated page.
            unsafe {
                let desc = &mut *rx_ring_virt.add(i);
                desc.addr = buf_phys;
                desc.status = 0;
            }
        }

        // Program RX ring registers.
        regs.set_rdbal((rx_ring_phys & 0xFFFF_FFFF) as u32);
        regs.set_rdbah((rx_ring_phys >> 32) as u32);
        regs.set_rdlen((RX_RING_SIZE * core::mem::size_of::<RxDesc>()) as u32);
        regs.set_rdh(0);
        regs.set_rdt((RX_RING_SIZE - 1) as u32);

        // Enable receiver: EN + BAM + SECRC, BSIZE bits 00 = 2048 bytes.
        regs.set_rctl(Rctl::EN | Rctl::BAM | Rctl::SECRC);

        // 10. Init TX ring.
        let tx_ring_phys = dma.alloc_frames(1).map_err(|_| DriverError::InitFailed)?;
        let tx_ring_virt = dma.phys_to_virt(tx_ring_phys) as *mut TxDesc;

        // Zero all TX descriptors and mark each as "done" (available).
        // SAFETY: Freshly allocated DMA page.
        unsafe {
            ptr::write_bytes(tx_ring_virt, 0, TX_RING_SIZE);
            for i in 0..TX_RING_SIZE {
                (*tx_ring_virt.add(i)).status = TXD_STAT_DD;
            }
        }

        // Program TX ring registers.
        regs.set_tdbal((tx_ring_phys & 0xFFFF_FFFF) as u32);
        regs.set_tdbah((tx_ring_phys >> 32) as u32);
        regs.set_tdlen((TX_RING_SIZE * core::mem::size_of::<TxDesc>()) as u32);
        regs.set_tdh(0);
        regs.set_tdt(0);

        // Enable transmitter: EN + PSP + CT=0x0F + COLD=0x200.
        let tctl_raw = Tctl::EN.bits()
            | Tctl::PSP.bits()
            | (0x0F << 4)    // Collision Threshold
            | (0x200 << 12); // Collision Distance
        regs.set_tctl(Tctl::from_bits_retain(tctl_raw));

        // Transmit IPG: IPGT=10, IPGR1=8, IPGR2=12.
        regs.set_tipg(10 | (8 << 10) | (12 << 20));

        // 11. Enable interrupts.
        if let Some(ref msix) = msix_table {
            msix.unmask(0);
            msix.enable();
        }
        regs.set_ims(ICR_TXDW | ICR_LSC | ICR_RXDMT | ICR_RXT0);

        // Check link status.
        let status = regs.status();
        if status.contains(regs::Status::LU) {
            hadron_kernel::kinfo!("e1000e: link up, device ready, irq vector {}", irq.vector());
        } else {
            hadron_kernel::kwarn!(
                "e1000e: link down (may come up later), irq vector {}",
                irq.vector()
            );
        }

        // 12. Build NIC struct and register.
        let nic = E1000eNic {
            regs,
            rx: SpinLock::named(
                "E1000e.rx",
                RxRingState {
                    ring_virt: rx_ring_virt,
                    tail: (RX_RING_SIZE - 1) as u16,
                },
            ),
            tx: SpinLock::named(
                "E1000e.tx",
                TxRingState {
                    ring_virt: tx_ring_virt,
                    tail: 0,
                },
            ),
            rx_bufs,
            irq,
            dma: *dma,
            mac,
        };

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
            "e1000e",
            idx,
        );
        devices.add_net_device(path, nic);

        hadron_kernel::kinfo!("e1000e: registered as \"e1000e-{}\"", idx);
        Ok(PciDriverRegistration {
            devices,
            lifecycle: None,
        })
    }
}
