//! VirtIO PCI modern transport.
//!
//! Locates VirtIO configuration structures via PCI capabilities and provides
//! MMIO-based access to common config, notify, ISR, and device-specific regions.

use crate::pci::cam::regs;
use crate::pci::caps::{self, MsixCapability, RawCapability, VirtioPciCap, VirtioPciCfgType};
use hadron_kernel::driver_api::capability::MmioCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::pci::{PciBar, PciDeviceInfo};
use hadron_kernel::driver_api::resource::MmioRegion;

/// Mapped VirtIO PCI configuration regions.
///
/// Holds MMIO pointers to the four VirtIO config structures discovered
/// through PCI vendor-specific capabilities.
pub struct VirtioPciTransport {
    /// Common configuration structure (device status, features, queue config).
    common: MmioRegion,
    /// Offset of common config within its BAR.
    common_offset: u32,
    /// Notify configuration structure (queue doorbell writes).
    notify: MmioRegion,
    /// Offset of notify config within its BAR.
    notify_offset: u32,
    /// Notify offset multiplier (from the notify capability).
    notify_off_multiplier: u32,
    /// ISR status structure.
    isr: MmioRegion,
    /// Offset of ISR config within its BAR.
    isr_offset: u32,
    /// Device-specific configuration structure.
    device_cfg: Option<(MmioRegion, u32)>,
    /// MSI-X capability, if present.
    msix_cap: Option<MsixCapability>,
}

impl VirtioPciTransport {
    /// Discovers VirtIO config structures via PCI capabilities and maps the BARs.
    pub fn new(info: &PciDeviceInfo, mmio_cap: &MmioCapability) -> Result<Self, DriverError> {
        let cap_iter = caps::walk_capabilities(&info.address).ok_or(DriverError::InitFailed)?;

        let mut common_cap: Option<VirtioPciCap> = None;
        let mut notify_cap: Option<VirtioPciCap> = None;
        let mut isr_cap: Option<VirtioPciCap> = None;
        let mut device_cap: Option<VirtioPciCap> = None;
        let mut msix_cap: Option<MsixCapability> = None;

        for RawCapability { id, offset } in cap_iter {
            match id {
                regs::CAP_ID_VENDOR => {
                    if let Some(vcap) = caps::read_virtio_pci_cap(&info.address, offset) {
                        match vcap.cfg_type {
                            VirtioPciCfgType::CommonCfg => common_cap = Some(vcap),
                            VirtioPciCfgType::NotifyCfg => notify_cap = Some(vcap),
                            VirtioPciCfgType::IsrCfg => isr_cap = Some(vcap),
                            VirtioPciCfgType::DeviceCfg => device_cap = Some(vcap),
                            VirtioPciCfgType::PciCfg => {} // not used in modern transport
                        }
                    }
                }
                regs::CAP_ID_MSIX => {
                    msix_cap = Some(caps::read_msix_cap(&info.address, offset));
                }
                _ => {}
            }
        }

        let common_cap = common_cap.ok_or(DriverError::InitFailed)?;
        let notify_cap = notify_cap.ok_or(DriverError::InitFailed)?;
        let isr_cap = isr_cap.ok_or(DriverError::InitFailed)?;

        // Read notify_off_multiplier from the notify capability (at cap_offset + 16).
        // SAFETY: Reading PCI config space of an enumerated device.
        let notify_off_multiplier = unsafe {
            crate::pci::cam::PciCam::read_u32(
                info.address.bus,
                info.address.device,
                info.address.function,
                notify_cap.cap_offset + 16,
            )
        };

        // Map BARs. We cache mapped BARs to avoid double-mapping.
        let mut bar_mmios: [Option<MmioRegion>; 6] = [None; 6];

        let map_bar = |bar_idx: u8,
                       bar_mmios: &mut [Option<MmioRegion>; 6]|
         -> Result<MmioRegion, DriverError> {
            if let Some(mmio) = bar_mmios[bar_idx as usize] {
                return Ok(mmio);
            }
            let (phys, size) = match info.bars[bar_idx as usize] {
                PciBar::Memory { base, size, .. } => (base, size),
                _ => return Err(DriverError::InitFailed),
            };
            let mmio = mmio_cap.map_mmio(phys, size)?;
            bar_mmios[bar_idx as usize] = Some(mmio);
            Ok(mmio)
        };

        let common_mmio = map_bar(common_cap.bar, &mut bar_mmios)?;
        let notify_mmio = map_bar(notify_cap.bar, &mut bar_mmios)?;
        let isr_mmio = map_bar(isr_cap.bar, &mut bar_mmios)?;

        let device_cfg = if let Some(ref dcap) = device_cap {
            let mmio = map_bar(dcap.bar, &mut bar_mmios)?;
            Some((mmio, dcap.offset))
        } else {
            None
        };

        Ok(Self {
            common: common_mmio,
            common_offset: common_cap.offset,
            notify: notify_mmio,
            notify_offset: notify_cap.offset,
            notify_off_multiplier,
            isr: isr_mmio,
            isr_offset: isr_cap.offset,
            device_cfg,
            msix_cap,
        })
    }

    /// Returns the MSI-X capability if the device supports it.
    #[must_use]
    pub fn msix_cap(&self) -> Option<&MsixCapability> {
        self.msix_cap.as_ref()
    }

    // -- Common config accessors ----------------------------------------------

    /// Reads a 8-bit value from the common config region at the given offset.
    ///
    /// # Safety
    ///
    /// `offset` must be within the common config structure bounds.
    unsafe fn common_read_u8(&self, offset: u32) -> u8 {
        let ptr = self
            .common
            .ptr_at(u64::from(self.common_offset + offset))
            .expect("common config offset out of bounds");
        // SAFETY: ptr is within the mapped MMIO region.
        unsafe { core::ptr::read_volatile(ptr) }
    }

    /// Writes a 8-bit value to the common config region.
    ///
    /// # Safety
    ///
    /// `offset` must be within the common config structure bounds.
    unsafe fn common_write_u8(&self, offset: u32, val: u8) {
        let ptr = self
            .common
            .ptr_at(u64::from(self.common_offset + offset))
            .expect("common config offset out of bounds");
        // SAFETY: ptr is within the mapped MMIO region.
        unsafe { core::ptr::write_volatile(ptr, val) }
    }

    /// Reads a 16-bit value from the common config region.
    ///
    /// # Safety
    ///
    /// `offset` must be within the common config structure bounds and
    /// properly aligned.
    unsafe fn common_read_u16(&self, offset: u32) -> u16 {
        let ptr = self
            .common
            .ptr_at(u64::from(self.common_offset + offset))
            .expect("common config offset out of bounds");
        // SAFETY: ptr is within the mapped MMIO region.
        unsafe { core::ptr::read_volatile(ptr.cast::<u16>()) }
    }

    /// Writes a 16-bit value to the common config region.
    ///
    /// # Safety
    ///
    /// `offset` must be within the common config structure bounds and
    /// properly aligned.
    unsafe fn common_write_u16(&self, offset: u32, val: u16) {
        let ptr = self
            .common
            .ptr_at(u64::from(self.common_offset + offset))
            .expect("common config offset out of bounds");
        // SAFETY: ptr is within the mapped MMIO region.
        unsafe { core::ptr::write_volatile(ptr.cast::<u16>(), val) }
    }

    /// Reads a 32-bit value from the common config region.
    ///
    /// # Safety
    ///
    /// `offset` must be within the common config structure bounds and
    /// properly aligned.
    unsafe fn common_read_u32(&self, offset: u32) -> u32 {
        let ptr = self
            .common
            .ptr_at(u64::from(self.common_offset + offset))
            .expect("common config offset out of bounds");
        // SAFETY: ptr is within the mapped MMIO region.
        unsafe { core::ptr::read_volatile(ptr.cast::<u32>()) }
    }

    /// Writes a 32-bit value to the common config region.
    ///
    /// # Safety
    ///
    /// `offset` must be within the common config structure bounds and
    /// properly aligned.
    unsafe fn common_write_u32(&self, offset: u32, val: u32) {
        let ptr = self
            .common
            .ptr_at(u64::from(self.common_offset + offset))
            .expect("common config offset out of bounds");
        // SAFETY: ptr is within the mapped MMIO region.
        unsafe { core::ptr::write_volatile(ptr.cast::<u32>(), val) }
    }

    // -- VirtIO common config fields ------------------------------------------
    // Offsets per VirtIO 1.0 spec §4.1.4.3

    /// Common config: `device_feature_select` (offset 0x00, 32-bit).
    pub fn set_device_feature_select(&self, val: u32) {
        // SAFETY: Offset 0x00 is within common config.
        unsafe { self.common_write_u32(0x00, val) }
    }

    /// Common config: `device_feature` (offset 0x04, 32-bit).
    pub fn device_feature(&self) -> u32 {
        // SAFETY: Offset 0x04 is within common config.
        unsafe { self.common_read_u32(0x04) }
    }

    /// Common config: `driver_feature_select` (offset 0x08, 32-bit).
    pub fn set_driver_feature_select(&self, val: u32) {
        // SAFETY: Offset 0x08 is within common config.
        unsafe { self.common_write_u32(0x08, val) }
    }

    /// Common config: `driver_feature` (offset 0x0C, 32-bit).
    pub fn set_driver_feature(&self, val: u32) {
        // SAFETY: Offset 0x0C is within common config.
        unsafe { self.common_write_u32(0x0C, val) }
    }

    /// Common config: `msix_config` (offset 0x10, 16-bit).
    pub fn set_msix_config(&self, val: u16) {
        // SAFETY: Offset 0x10 is within common config.
        unsafe { self.common_write_u16(0x10, val) }
    }

    /// Common config: `num_queues` (offset 0x12, 16-bit).
    pub fn num_queues(&self) -> u16 {
        // SAFETY: Offset 0x12 is within common config.
        unsafe { self.common_read_u16(0x12) }
    }

    /// Common config: `device_status` (offset 0x14, 8-bit).
    pub fn device_status(&self) -> u8 {
        // SAFETY: Offset 0x14 is within common config.
        unsafe { self.common_read_u8(0x14) }
    }

    /// Common config: write `device_status` (offset 0x14, 8-bit).
    pub fn set_device_status(&self, val: u8) {
        // SAFETY: Offset 0x14 is within common config.
        unsafe { self.common_write_u8(0x14, val) }
    }

    /// Common config: `queue_select` (offset 0x16, 16-bit).
    pub fn set_queue_select(&self, val: u16) {
        // SAFETY: Offset 0x16 is within common config.
        unsafe { self.common_write_u16(0x16, val) }
    }

    /// Common config: `queue_size` (offset 0x18, 16-bit).
    pub fn queue_size(&self) -> u16 {
        // SAFETY: Offset 0x18 is within common config.
        unsafe { self.common_read_u16(0x18) }
    }

    /// Common config: write `queue_size` (offset 0x18, 16-bit).
    pub fn set_queue_size(&self, val: u16) {
        // SAFETY: Offset 0x18 is within common config.
        unsafe { self.common_write_u16(0x18, val) }
    }

    /// Common config: `queue_msix_vector` (offset 0x1A, 16-bit).
    pub fn set_queue_msix_vector(&self, val: u16) {
        // SAFETY: Offset 0x1A is within common config.
        unsafe { self.common_write_u16(0x1A, val) }
    }

    /// Common config: read `queue_msix_vector` (offset 0x1A, 16-bit).
    pub fn queue_msix_vector(&self) -> u16 {
        // SAFETY: Offset 0x1A is within common config.
        unsafe { self.common_read_u16(0x1A) }
    }

    /// Common config: `queue_enable` (offset 0x1C, 16-bit).
    pub fn set_queue_enable(&self, val: u16) {
        // SAFETY: Offset 0x1C is within common config.
        unsafe { self.common_write_u16(0x1C, val) }
    }

    /// Common config: `queue_notify_off` (offset 0x1E, 16-bit).
    pub fn queue_notify_off(&self) -> u16 {
        // SAFETY: Offset 0x1E is within common config.
        unsafe { self.common_read_u16(0x1E) }
    }

    /// Common config: `queue_desc` (offset 0x20, 64-bit, written as two 32-bit).
    pub fn set_queue_desc(&self, addr: u64) {
        // SAFETY: Offsets 0x20 and 0x24 are within common config.
        unsafe {
            self.common_write_u32(0x20, addr as u32);
            self.common_write_u32(0x24, (addr >> 32) as u32);
        }
    }

    /// Common config: `queue_avail` (offset 0x28, 64-bit).
    pub fn set_queue_avail(&self, addr: u64) {
        // SAFETY: Offsets 0x28 and 0x2C are within common config.
        unsafe {
            self.common_write_u32(0x28, addr as u32);
            self.common_write_u32(0x2C, (addr >> 32) as u32);
        }
    }

    /// Common config: `queue_used` (offset 0x30, 64-bit).
    pub fn set_queue_used(&self, addr: u64) {
        // SAFETY: Offsets 0x30 and 0x34 are within common config.
        unsafe {
            self.common_write_u32(0x30, addr as u32);
            self.common_write_u32(0x34, (addr >> 32) as u32);
        }
    }

    // -- Notify ---------------------------------------------------------------

    /// Writes to the queue notify doorbell for the given queue index.
    pub fn notify_queue(&self, queue_index: u16) {
        let notify_off = {
            // First select the queue to read its notify_off.
            self.set_queue_select(queue_index);
            self.queue_notify_off()
        };

        let offset = self.notify_offset + u32::from(notify_off) * self.notify_off_multiplier;

        let ptr = self
            .notify
            .ptr_at(u64::from(offset))
            .expect("notify offset out of bounds");
        // SAFETY: ptr is within the mapped notify MMIO region.
        unsafe { core::ptr::write_volatile(ptr.cast::<u16>(), queue_index) };
    }

    // -- ISR ------------------------------------------------------------------

    /// Reads the ISR status register (clears on read).
    pub fn isr_status(&self) -> u8 {
        let ptr = self
            .isr
            .ptr_at(u64::from(self.isr_offset))
            .expect("ISR offset out of bounds");
        // SAFETY: ptr is within the mapped ISR MMIO region.
        unsafe { core::ptr::read_volatile(ptr) }
    }

    // -- Device config --------------------------------------------------------

    /// Reads a 32-bit value from the device-specific config region.
    ///
    /// Returns `None` if the device has no device-specific config.
    pub fn device_cfg_read_u32(&self, offset: u32) -> Option<u32> {
        let (ref mmio, base_offset) = *self.device_cfg.as_ref()?;
        let ptr = mmio
            .ptr_at(u64::from(base_offset + offset))
            .expect("device config offset out of bounds");
        // SAFETY: ptr is within the mapped device config MMIO region.
        Some(unsafe { core::ptr::read_volatile(ptr.cast::<u32>()) })
    }

    /// Reads a 64-bit value from the device-specific config region (as two 32-bit reads).
    pub fn device_cfg_read_u64(&self, offset: u32) -> Option<u64> {
        let lo = self.device_cfg_read_u32(offset)? as u64;
        let hi = self.device_cfg_read_u32(offset + 4)? as u64;
        Some(lo | (hi << 32))
    }
}
