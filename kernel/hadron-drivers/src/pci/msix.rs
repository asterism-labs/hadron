//! MSI-X interrupt support for PCI devices.
//!
//! Provides [`MsixTable`] which maps and configures MSI-X table entries,
//! allowing PCI devices to use message-signaled interrupts instead of
//! legacy INTx lines.

use super::cam::PciCam;
use super::caps::MsixCapability;
use hadron_kernel::driver_api::capability::MmioCapability;
use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::pci::{PciBar, PciDeviceInfo};
use hadron_kernel::driver_api::resource::MmioRegion;

/// MSI-X table entry size in bytes.
const MSIX_ENTRY_SIZE: u32 = 16;

/// MSI-X message address base for x86 (fixed at 0xFEE0_0000).
const MSIX_MSG_ADDR_BASE: u32 = 0xFEE0_0000;

/// MSI-X Message Control register: MSI-X Enable (bit 15).
const MSIX_CTRL_ENABLE: u16 = 1 << 15;
/// MSI-X Message Control register: Function Mask (bit 14).
const MSIX_CTRL_FUNCTION_MASK: u16 = 1 << 14;

// -- MSI-X table entry offsets ------------------------------------------------

/// Offset of `msg_addr_lo` within an MSI-X table entry.
const ENTRY_MSG_ADDR_LO: u32 = 0;
/// Offset of `msg_addr_hi` within an MSI-X table entry.
const ENTRY_MSG_ADDR_HI: u32 = 4;
/// Offset of `msg_data` within an MSI-X table entry.
const ENTRY_MSG_DATA: u32 = 8;
/// Offset of `vector_ctrl` within an MSI-X table entry.
const ENTRY_VECTOR_CTRL: u32 = 12;

/// Mapped MSI-X table for a PCI device.
///
/// Provides methods to configure individual MSI-X entries with target
/// CPU and vector, and to mask/unmask entries.
pub struct MsixTable {
    /// MMIO region containing the MSI-X table.
    mmio: MmioRegion,
    /// Byte offset of the table within the MMIO region.
    table_offset: u32,
    /// Number of MSI-X entries (table_size field + 1).
    entry_count: u16,
    /// Config-space offset of the MSI-X capability (for enable/disable).
    cap_offset: u8,
    /// PCI BDF address for config-space writes.
    bus: u8,
    device: u8,
    function: u8,
}

impl MsixTable {
    /// Sets up MSI-X for a PCI device.
    ///
    /// Maps the BAR containing the MSI-X table, enables MSI-X in the PCI
    /// config space, and returns a handle to the mapped table.
    pub fn setup(
        info: &PciDeviceInfo,
        cap: &MsixCapability,
        mmio_cap: &MmioCapability,
    ) -> Result<Self, DriverError> {
        // Extract the BAR that contains the MSI-X table.
        let (bar_phys, bar_size) = match info.bars[cap.table_bar as usize] {
            PciBar::Memory { base, size, .. } => (base, size),
            _ => return Err(DriverError::InitFailed),
        };

        // Map the BAR.
        let mmio = mmio_cap.map_mmio(bar_phys, bar_size)?;

        let table = Self {
            mmio,
            table_offset: cap.table_offset,
            entry_count: cap.table_size + 1,
            cap_offset: cap.cap_offset,
            bus: info.address.bus,
            device: info.address.device,
            function: info.address.function,
        };

        // Enable MSI-X and set function mask while configuring.
        table.write_msg_control(MSIX_CTRL_ENABLE | MSIX_CTRL_FUNCTION_MASK);

        Ok(table)
    }

    /// Configures an MSI-X table entry to target the given CPU and vector.
    ///
    /// The entry is left masked; call [`unmask`](Self::unmask) after setup.
    pub fn set_entry(&self, index: u16, vector: u8, cpu: u8) {
        let base = self.entry_offset(index);

        let addr_lo = MSIX_MSG_ADDR_BASE | (u32::from(cpu) << 12);

        // SAFETY: Writing to MMIO-mapped MSI-X table within bounds verified
        // by entry_offset.
        unsafe {
            self.write_entry_u32(base + ENTRY_MSG_ADDR_LO, addr_lo);
            self.write_entry_u32(base + ENTRY_MSG_ADDR_HI, 0);
            self.write_entry_u32(base + ENTRY_MSG_DATA, u32::from(vector));
            // Mask the entry (bit 0 = 1).
            self.write_entry_u32(base + ENTRY_VECTOR_CTRL, 1);
        }
    }

    /// Unmasks an MSI-X table entry, allowing it to generate interrupts.
    pub fn unmask(&self, index: u16) {
        let base = self.entry_offset(index);
        // SAFETY: Writing to MMIO-mapped MSI-X table entry.
        unsafe {
            self.write_entry_u32(base + ENTRY_VECTOR_CTRL, 0);
        }
    }

    /// Clears the function mask, allowing all configured entries to fire.
    ///
    /// Call this after all entries are configured and unmasked.
    pub fn enable(&self) {
        self.write_msg_control(MSIX_CTRL_ENABLE);
    }

    /// Returns the number of MSI-X table entries.
    #[must_use]
    pub fn entry_count(&self) -> u16 {
        self.entry_count
    }

    /// Returns the MMIO region backing this MSI-X table.
    ///
    /// Useful when the MSI-X BAR is shared with device registers (common
    /// with VirtIO), avoiding a duplicate mapping.
    #[must_use]
    pub fn mmio(&self) -> &MmioRegion {
        &self.mmio
    }

    // -- Internal helpers -----------------------------------------------------

    /// Computes the byte offset of an entry within the MMIO region.
    fn entry_offset(&self, index: u16) -> u32 {
        self.table_offset + u32::from(index) * MSIX_ENTRY_SIZE
    }

    /// Writes a 32-bit value to the MSI-X table at the given byte offset.
    ///
    /// # Safety
    ///
    /// The caller must ensure `offset` is within the mapped MMIO region.
    unsafe fn write_entry_u32(&self, offset: u32, val: u32) {
        let ptr = self
            .mmio
            .ptr_at(u64::from(offset))
            .expect("MSI-X table entry offset out of bounds");
        // SAFETY: ptr_at returned a valid pointer within the MMIO mapping.
        unsafe { core::ptr::write_volatile(ptr.cast::<u32>(), val) };
    }

    /// Writes the MSI-X Message Control register in PCI config space.
    fn write_msg_control(&self, value: u16) {
        // Message Control is at cap_offset + 2 (16-bit).
        // We need to do a read-modify-write of the dword containing it.
        let dword_offset = self.cap_offset & 0xFC;
        // SAFETY: Writing PCI config space for an enumerated device.
        unsafe {
            let dword = PciCam::read_u32(self.bus, self.device, self.function, dword_offset);
            // Message Control is the upper 16 bits of the dword at cap_offset
            // (cap_offset is dword-aligned, message control at +2).
            let new_dword = (dword & 0x0000_FFFF) | (u32::from(value) << 16);
            PciCam::write_u32(self.bus, self.device, self.function, dword_offset, new_dword);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msix_msg_addr_encoding() {
        // CPU 0, vector 48
        let addr = MSIX_MSG_ADDR_BASE | (u32::from(0u8) << 12);
        assert_eq!(addr, 0xFEE0_0000);

        // CPU 3, vector 48
        let addr = MSIX_MSG_ADDR_BASE | (u32::from(3u8) << 12);
        assert_eq!(addr, 0xFEE0_3000);
    }

    #[test]
    fn msix_entry_offset_calculation() {
        // Entry 0 at table_offset 0x2000
        let offset = 0x2000u32 + u32::from(0u16) * MSIX_ENTRY_SIZE;
        assert_eq!(offset, 0x2000);

        // Entry 5 at table_offset 0x2000
        let offset = 0x2000u32 + u32::from(5u16) * MSIX_ENTRY_SIZE;
        assert_eq!(offset, 0x2000 + 80);
    }
}
