//! AHCI per-port state and command management.
//!
//! Each AHCI port represents a SATA device connection. This module handles
//! port initialization (command list, FIS buffer, IDENTIFY), command slot
//! management, and async I/O submission.

use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_kernel::addr::VirtAddr;
use hadron_kernel::driver_api::block::IoError;
use hadron_kernel::driver_api::services::KernelServices;

use super::command::{
    CMD_FIS_LEN_DWORDS, CMD_FIS_OFFSET, CommandHeader, FisRegH2d, PRDT_OFFSET, PrdtEntry,
};
use super::hba::AhciHba;
use super::regs::{self, FIS_TYPE_REG_H2D, PortCmd, PortIe, PortIs};
use super::regs::{ATA_CMD_IDENTIFY, ATA_CMD_READ_DMA_EX, SSTS_DET_PRESENT, SSTS_IPM_ACTIVE};

/// Page size for DMA allocations.
const PAGE_SIZE: u64 = 4096;

/// Size of the command list (32 headers * 32 bytes = 1024 bytes).
const CMD_LIST_SIZE: u64 = 32 * 32;

/// Maximum spin iterations when waiting for port status bits.
const PORT_SPIN_TIMEOUT: u32 = 1_000_000;

/// ATA sector size in bytes.
const ATA_SECTOR_SIZE: usize = 512;

/// Parsed IDENTIFY DEVICE data.
pub struct DeviceIdentity {
    /// Total addressable sectors (48-bit LBA).
    pub sector_count: u64,
    /// Sector size in bytes.
    pub sector_size: usize,
    /// Model string (40 bytes, ATA byte-swapped).
    pub model: [u8; 40],
    /// Serial number (20 bytes, ATA byte-swapped).
    pub serial: [u8; 20],
}

/// Per-port AHCI state.
pub struct AhciPort {
    /// Virtual base address of this port's register block.
    port_base: VirtAddr,
    /// Port number (0-31).
    port_num: u8,
    /// Number of command slots supported by the HBA.
    num_cmd_slots: u8,
    /// Physical address of the CLB+FB DMA page.
    #[allow(dead_code, reason = "needed for Drop cleanup in Phase 10")]
    clb_fb_phys: u64,
    /// Virtual address of the CLB+FB DMA page.
    clb_fb_virt: u64,
    /// Physical addresses of per-slot command tables.
    #[allow(dead_code, reason = "needed for Drop cleanup in Phase 10")]
    cmd_table_phys: [u64; 32],
    /// Virtual addresses of per-slot command tables.
    cmd_table_virt: [u64; 32],
    /// Bitmask of command slots currently in use.
    slots_in_use: AtomicU32,
    /// Parsed device identity, populated after IDENTIFY.
    pub identity: Option<DeviceIdentity>,
}

// SAFETY: AhciPort fields are either plain data or atomics. The raw pointers
// derived from VirtAddr/u64 are only used for volatile MMIO access which is
// inherently shared-state safe.
unsafe impl Send for AhciPort {}
unsafe impl Sync for AhciPort {}

impl AhciPort {
    /// Initializes an AHCI port: allocates DMA buffers, sets up command list
    /// and FIS structures, runs IDENTIFY DEVICE.
    ///
    /// Returns `None` if the port has no device connected.
    pub fn init(
        hba: &AhciHba,
        port_num: u8,
        services: &'static dyn KernelServices,
    ) -> Option<Self> {
        let port_base = hba.port_base(port_num);

        // Check device presence via SStatus.
        let ssts = volatile_read32(port_base, regs::PORT_SSTS);
        if regs::ssts_det(ssts) != SSTS_DET_PRESENT || regs::ssts_ipm(ssts) != SSTS_IPM_ACTIVE {
            return None;
        }

        // Stop command engine before reconfiguring.
        stop_command_engine(port_base);

        // Allocate one page for CLB (1024 bytes) + received FIS (256 bytes).
        let clb_fb_phys = services
            .alloc_dma_frames(1)
            .expect("AHCI: failed to allocate CLB/FB page");
        let clb_fb_virt = services.phys_to_virt(clb_fb_phys);

        // Zero-initialize the CLB/FB page.
        // SAFETY: We just allocated and mapped this page.
        unsafe { ptr::write_bytes(clb_fb_virt as *mut u8, 0, PAGE_SIZE as usize) };

        // Write CLB and FB physical addresses to port registers.
        volatile_write32(port_base, regs::PORT_CLB, clb_fb_phys as u32);
        volatile_write32(port_base, regs::PORT_CLBU, (clb_fb_phys >> 32) as u32);

        let fb_phys = clb_fb_phys + CMD_LIST_SIZE;
        volatile_write32(port_base, regs::PORT_FB, fb_phys as u32);
        volatile_write32(port_base, regs::PORT_FBU, (fb_phys >> 32) as u32);

        // Allocate per-slot command tables (one page each for simplicity).
        let num_slots = hba.num_cmd_slots;
        let mut cmd_table_phys = [0u64; 32];
        let mut cmd_table_virt = [0u64; 32];

        for slot in 0..num_slots as usize {
            let ct_phys = services
                .alloc_dma_frames(1)
                .expect("AHCI: failed to allocate command table");
            let ct_virt = services.phys_to_virt(ct_phys);
            // SAFETY: Newly allocated page.
            unsafe { ptr::write_bytes(ct_virt as *mut u8, 0, PAGE_SIZE as usize) };

            cmd_table_phys[slot] = ct_phys;
            cmd_table_virt[slot] = ct_virt;

            // Set CTBA/CTBAU in the command header.
            let header_ptr = (clb_fb_virt
                + (slot as u64) * (core::mem::size_of::<CommandHeader>() as u64))
                as *mut CommandHeader;
            // SAFETY: Writing to our own DMA buffer within the CLB page.
            unsafe {
                let mut hdr = ptr::read_volatile(header_ptr);
                hdr.ctba = ct_phys as u32;
                hdr.ctbau = (ct_phys >> 32) as u32;
                ptr::write_volatile(header_ptr, hdr);
            }
        }

        // Clear SERR and configure interrupts.
        volatile_write32(port_base, regs::PORT_SERR, 0xFFFF_FFFF);
        let ie = PortIe::DHRE | PortIe::TFEE;
        volatile_write32(port_base, regs::PORT_IE, ie.bits());

        // Start command engine.
        start_command_engine(port_base);

        let mut port = Self {
            port_base,
            port_num,
            num_cmd_slots: num_slots,
            clb_fb_phys,
            clb_fb_virt,
            cmd_table_phys,
            cmd_table_virt,
            slots_in_use: AtomicU32::new(0),
            identity: None,
        };

        // Run IDENTIFY DEVICE.
        if let Ok(ident) = port.identify_device(services) {
            hadron_kernel::kinfo!(
                "AHCI: port {} -- {} sectors, {} bytes/sector",
                port_num,
                ident.sector_count,
                ident.sector_size
            );
            port.identity = Some(ident);
        }

        Some(port)
    }

    /// Allocates a free command slot.
    pub fn alloc_slot(&self) -> Result<u8, IoError> {
        let mask = if self.num_cmd_slots >= 32 {
            u32::MAX
        } else {
            (1u32 << self.num_cmd_slots) - 1
        };

        loop {
            let in_use = self.slots_in_use.load(Ordering::Acquire);
            let free = !in_use & mask;
            if free == 0 {
                return Err(IoError::NotReady);
            }
            let slot = free.trailing_zeros() as u8;
            let bit = 1u32 << slot;
            if self
                .slots_in_use
                .compare_exchange_weak(in_use, in_use | bit, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(slot);
            }
        }
    }

    /// Releases a command slot.
    pub fn free_slot(&self, slot: u8) {
        let bit = 1u32 << slot;
        self.slots_in_use.fetch_and(!bit, Ordering::Release);
    }

    /// Runs IDENTIFY DEVICE on this port (blocking poll).
    fn identify_device(
        &mut self,
        services: &'static dyn KernelServices,
    ) -> Result<DeviceIdentity, IoError> {
        let slot = self.alloc_slot()?;

        // Allocate a DMA buffer for the 512-byte IDENTIFY response.
        let buf_phys = services
            .alloc_dma_frames(1)
            .map_err(|_| IoError::DmaError)?;
        let buf_virt = services.phys_to_virt(buf_phys);
        // SAFETY: Freshly allocated page.
        unsafe { ptr::write_bytes(buf_virt as *mut u8, 0, PAGE_SIZE as usize) };

        // Build command FIS.
        self.setup_command(
            slot,
            ATA_CMD_IDENTIFY,
            0,
            1,
            buf_phys,
            ATA_SECTOR_SIZE,
            false,
        );

        // Issue command and poll for completion.
        self.issue_command_poll(slot)?;
        self.free_slot(slot);

        // Parse IDENTIFY response.
        let data = buf_virt as *const u16;
        let ident = parse_identify(data);

        // Free the DMA buffer.
        // SAFETY: buf_phys was allocated by us and is no longer referenced.
        unsafe { services.free_dma_frames(buf_phys, 1) };

        Ok(ident)
    }

    /// Sets up a command in the given slot.
    fn setup_command(
        &self,
        slot: u8,
        command: u8,
        lba: u64,
        sector_count: u16,
        dma_phys: u64,
        byte_count: usize,
        write: bool,
    ) {
        let ct_virt = self.cmd_table_virt[slot as usize];

        // Build FIS Register H2D at the start of the command table.
        let fis_ptr = (ct_virt + CMD_FIS_OFFSET as u64) as *mut FisRegH2d;
        let mut fis = FisRegH2d::zeroed();
        fis.fis_type = FIS_TYPE_REG_H2D;
        fis.pm_and_c = 0x80; // C bit = 1 (command)
        fis.command = command;
        fis.device = 1 << 6; // LBA mode
        fis.lba0 = lba as u8;
        fis.lba1 = (lba >> 8) as u8;
        fis.lba2 = (lba >> 16) as u8;
        fis.lba3 = (lba >> 24) as u8;
        fis.lba4 = (lba >> 32) as u8;
        fis.lba5 = (lba >> 40) as u8;
        fis.count_lo = sector_count as u8;
        fis.count_hi = (sector_count >> 8) as u8;
        // SAFETY: ct_virt is our own DMA buffer.
        unsafe { ptr::write_volatile(fis_ptr, fis) };

        // Build PRDT entry.
        let prdt_ptr = (ct_virt + PRDT_OFFSET as u64) as *mut PrdtEntry;
        let prdt = PrdtEntry {
            dba: dma_phys as u32,
            dbau: (dma_phys >> 32) as u32,
            _reserved: 0,
            // byte_count - 1, with Interrupt on Completion bit (bit 31).
            dbc: ((byte_count as u32).saturating_sub(1)) | (1 << 31),
        };
        // SAFETY: ct_virt is our own DMA buffer.
        unsafe { ptr::write_volatile(prdt_ptr, prdt) };

        // Update command header.
        let header_ptr = (self.clb_fb_virt
            + u64::from(slot) * (core::mem::size_of::<CommandHeader>() as u64))
            as *mut CommandHeader;
        // SAFETY: CLB is our own DMA buffer.
        unsafe {
            let mut hdr = ptr::read_volatile(header_ptr);
            // CFL = 5 DWORDs, W bit = write, clear other flags.
            let mut flags = CMD_FIS_LEN_DWORDS;
            if write {
                flags |= 1 << 6; // W bit
            }
            hdr.flags = flags;
            hdr.prdtl = 1;
            hdr.prdbc = 0;
            ptr::write_volatile(header_ptr, hdr);
        }
    }

    /// Issues a command and polls for completion (blocking).
    fn issue_command_poll(&self, slot: u8) -> Result<(), IoError> {
        let ci_bit = 1u32 << slot;

        // Clear port IS.
        volatile_write32(self.port_base, regs::PORT_IS, 0xFFFF_FFFF);

        // Issue command.
        volatile_write32(self.port_base, regs::PORT_CI, ci_bit);

        // Poll for completion.
        for _ in 0..PORT_SPIN_TIMEOUT {
            let is = PortIs::from_bits_retain(volatile_read32(self.port_base, regs::PORT_IS));
            if is.contains(PortIs::TFES) {
                return Err(IoError::DeviceError);
            }
            let ci = volatile_read32(self.port_base, regs::PORT_CI);
            if ci & ci_bit == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }

        Err(IoError::Timeout)
    }

    /// Issues a command and waits for completion via async IRQ.
    pub async fn issue_command_async(
        &self,
        slot: u8,
        irq: &hadron_kernel::drivers::irq::IrqLine,
    ) -> Result<(), IoError> {
        let ci_bit = 1u32 << slot;

        // Clear port IS.
        volatile_write32(self.port_base, regs::PORT_IS, 0xFFFF_FFFF);

        // Issue command.
        volatile_write32(self.port_base, regs::PORT_CI, ci_bit);

        // Wait for IRQ, then check completion.
        loop {
            irq.wait().await;

            let is = PortIs::from_bits_retain(volatile_read32(self.port_base, regs::PORT_IS));

            if is.contains(PortIs::TFES) {
                self.free_slot(slot);
                return Err(IoError::DeviceError);
            }

            if is.contains(PortIs::DHRS) || is.contains(PortIs::PSS) {
                // Clear handled interrupt bits.
                volatile_write32(self.port_base, regs::PORT_IS, is.bits());

                let ci = volatile_read32(self.port_base, regs::PORT_CI);
                if ci & ci_bit == 0 {
                    return Ok(());
                }
            }
        }
    }

    /// Reads a sector via DMA into the provided physical buffer.
    pub fn setup_read_dma(
        &self,
        slot: u8,
        sector: u64,
        count: u16,
        dma_phys: u64,
        byte_count: usize,
    ) {
        self.setup_command(
            slot,
            ATA_CMD_READ_DMA_EX,
            sector,
            count,
            dma_phys,
            byte_count,
            false,
        );
    }

    /// Returns port number.
    #[must_use]
    pub const fn port_num(&self) -> u8 {
        self.port_num
    }
}

// ---------------------------------------------------------------------------
// IDENTIFY response parsing
// ---------------------------------------------------------------------------

/// Parses a 512-byte IDENTIFY DEVICE response.
///
/// # Safety
///
/// `data` must point to at least 256 valid `u16` values.
fn parse_identify(data: *const u16) -> DeviceIdentity {
    // SAFETY: Caller guarantees data points to 256 u16 values.
    let word = |idx: usize| -> u16 { unsafe { ptr::read_volatile(data.add(idx)) } };

    // Words 100-103: Total addressable sectors (48-bit LBA).
    let sector_count = u64::from(word(100))
        | (u64::from(word(101)) << 16)
        | (u64::from(word(102)) << 32)
        | (u64::from(word(103)) << 48);

    // Word 106: Physical/Logical sector size.
    let w106 = word(106);
    let sector_size = if w106 & (1 << 12) != 0 && w106 & (1 << 14) == 0 {
        // Logical sectors per physical sector.
        let exp = (w106 & 0x0F) as u32;
        ATA_SECTOR_SIZE * (1 << exp)
    } else {
        ATA_SECTOR_SIZE
    };

    // Words 27-46: Model number (ATA byte-swapped).
    let mut model = [0u8; 40];
    for i in 0..20 {
        let w = word(27 + i);
        model[i * 2] = (w >> 8) as u8;
        model[i * 2 + 1] = w as u8;
    }

    // Words 10-19: Serial number (ATA byte-swapped).
    let mut serial = [0u8; 20];
    for i in 0..10 {
        let w = word(10 + i);
        serial[i * 2] = (w >> 8) as u8;
        serial[i * 2 + 1] = w as u8;
    }

    DeviceIdentity {
        sector_count,
        sector_size,
        model,
        serial,
    }
}

// ---------------------------------------------------------------------------
// Port engine control helpers
// ---------------------------------------------------------------------------

/// Stops the command engine on a port.
fn stop_command_engine(port_base: VirtAddr) {
    let cmd = PortCmd::from_bits_retain(volatile_read32(port_base, regs::PORT_CMD));

    // If already stopped, nothing to do.
    if !cmd.contains(PortCmd::ST) && !cmd.contains(PortCmd::FRE) {
        return;
    }

    // Clear ST.
    let new_cmd = cmd.bits() & !PortCmd::ST.bits();
    volatile_write32(port_base, regs::PORT_CMD, new_cmd);

    // Wait for CR to clear.
    for _ in 0..PORT_SPIN_TIMEOUT {
        let cmd = PortCmd::from_bits_retain(volatile_read32(port_base, regs::PORT_CMD));
        if !cmd.contains(PortCmd::CR) {
            break;
        }
        core::hint::spin_loop();
    }

    // Clear FRE.
    let cmd = volatile_read32(port_base, regs::PORT_CMD);
    volatile_write32(port_base, regs::PORT_CMD, cmd & !PortCmd::FRE.bits());

    // Wait for FR to clear.
    for _ in 0..PORT_SPIN_TIMEOUT {
        let cmd = PortCmd::from_bits_retain(volatile_read32(port_base, regs::PORT_CMD));
        if !cmd.contains(PortCmd::FR) {
            break;
        }
        core::hint::spin_loop();
    }
}

/// Starts the command engine on a port.
fn start_command_engine(port_base: VirtAddr) {
    // Wait until CR is clear before starting.
    for _ in 0..PORT_SPIN_TIMEOUT {
        let cmd = PortCmd::from_bits_retain(volatile_read32(port_base, regs::PORT_CMD));
        if !cmd.contains(PortCmd::CR) {
            break;
        }
        core::hint::spin_loop();
    }

    // Set FRE first, then ST.
    let cmd = volatile_read32(port_base, regs::PORT_CMD);
    volatile_write32(port_base, regs::PORT_CMD, cmd | PortCmd::FRE.bits());

    // Wait for FR to be set.
    for _ in 0..PORT_SPIN_TIMEOUT {
        let cmd = PortCmd::from_bits_retain(volatile_read32(port_base, regs::PORT_CMD));
        if cmd.contains(PortCmd::FR) {
            break;
        }
        core::hint::spin_loop();
    }

    let cmd = volatile_read32(port_base, regs::PORT_CMD);
    volatile_write32(port_base, regs::PORT_CMD, cmd | PortCmd::ST.bits());
}

// ---------------------------------------------------------------------------
// Volatile MMIO helpers
// ---------------------------------------------------------------------------

/// Reads a 32-bit value from a port register.
fn volatile_read32(port_base: VirtAddr, offset: u64) -> u32 {
    let addr = (port_base.as_u64() + offset) as *const u32;
    // SAFETY: port_base is within the mapped AHCI MMIO region.
    unsafe { ptr::read_volatile(addr) }
}

/// Writes a 32-bit value to a port register.
fn volatile_write32(port_base: VirtAddr, offset: u64, value: u32) {
    let addr = (port_base.as_u64() + offset) as *mut u32;
    // SAFETY: port_base is within the mapped AHCI MMIO region.
    unsafe { ptr::write_volatile(addr, value) };
}
