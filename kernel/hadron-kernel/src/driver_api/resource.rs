//! Hardware resource types representing exclusive claims on I/O ports, MMIO regions, and IRQ lines.

use crate::addr::{PhysAddr, VirtAddr};

/// An exclusive claim on a contiguous range of x86 I/O ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoPortRange {
    base: u16,
    size: u16,
}

impl IoPortRange {
    /// Creates a new I/O port range.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the port range `[base, base + size)` is valid
    /// and not claimed by another driver.
    #[must_use]
    pub const unsafe fn new(base: u16, size: u16) -> Self {
        Self { base, size }
    }

    /// Returns the base I/O port address.
    #[must_use]
    pub const fn base(&self) -> u16 {
        self.base
    }

    /// Returns the number of ports in this range.
    #[must_use]
    pub const fn size(&self) -> u16 {
        self.size
    }

    /// Returns `true` if `offset` is within this range.
    #[must_use]
    pub const fn contains_offset(&self, offset: u16) -> bool {
        offset < self.size
    }

    /// Returns the absolute port number at the given offset from base.
    ///
    /// Returns `None` if the offset is out of range.
    #[must_use]
    pub const fn port_at(&self, offset: u16) -> Option<u16> {
        if offset < self.size {
            Some(self.base + offset)
        } else {
            None
        }
    }
}

/// An exclusive claim on a memory-mapped I/O region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmioRegion {
    phys_base: PhysAddr,
    virt_base: VirtAddr,
    size: u64,
}

impl MmioRegion {
    /// Creates a new MMIO region descriptor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `phys_base` and `virt_base` refer to the same physical region.
    /// - The region is not claimed by another driver.
    /// - The virtual mapping is valid for the lifetime of the region.
    #[must_use]
    pub const unsafe fn new(phys_base: PhysAddr, virt_base: VirtAddr, size: u64) -> Self {
        Self {
            phys_base,
            virt_base,
            size,
        }
    }

    /// Returns the physical base address.
    #[must_use]
    pub const fn phys_base(&self) -> PhysAddr {
        self.phys_base
    }

    /// Returns the virtual base address.
    #[must_use]
    pub const fn virt_base(&self) -> VirtAddr {
        self.virt_base
    }

    /// Returns the size of the region in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns a pointer to the given byte offset within the region.
    ///
    /// Returns `None` if the offset is out of bounds.
    #[must_use]
    pub const fn ptr_at(&self, offset: u64) -> Option<*mut u8> {
        if offset < self.size {
            Some((self.virt_base.as_u64() + offset) as *mut u8)
        } else {
            None
        }
    }
}

/// An exclusive claim on an interrupt request line (Global System Interrupt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrqLine {
    gsi: u32,
}

impl IrqLine {
    /// Creates a new IRQ line descriptor.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the GSI number is valid and not claimed
    /// by another driver.
    #[must_use]
    pub const unsafe fn new(gsi: u32) -> Self {
        Self { gsi }
    }

    /// Returns the Global System Interrupt number.
    #[must_use]
    pub const fn gsi(&self) -> u32 {
        self.gsi
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_port_range_basics() {
        // SAFETY: test-only, no real hardware.
        let range = unsafe { IoPortRange::new(0x3F8, 8) };
        assert_eq!(range.base(), 0x3F8);
        assert_eq!(range.size(), 8);
    }

    #[test]
    fn io_port_range_contains_offset() {
        let range = unsafe { IoPortRange::new(0x3F8, 8) };
        assert!(range.contains_offset(0));
        assert!(range.contains_offset(7));
        assert!(!range.contains_offset(8));
    }

    #[test]
    fn io_port_range_port_at() {
        let range = unsafe { IoPortRange::new(0x3F8, 8) };
        assert_eq!(range.port_at(0), Some(0x3F8));
        assert_eq!(range.port_at(7), Some(0x3FF));
        assert_eq!(range.port_at(8), None);
    }

    #[test]
    fn mmio_region_basics() {
        let phys = PhysAddr::new(0x1000);
        let virt = VirtAddr::new(0x1000);
        // SAFETY: test-only, no real hardware.
        let region = unsafe { MmioRegion::new(phys, virt, 4096) };
        assert_eq!(region.phys_base(), phys);
        assert_eq!(region.virt_base(), virt);
        assert_eq!(region.size(), 4096);
    }

    #[test]
    fn mmio_region_ptr_at() {
        let phys = PhysAddr::new(0x1000);
        let virt = VirtAddr::new(0x1000);
        let region = unsafe { MmioRegion::new(phys, virt, 4096) };
        assert!(region.ptr_at(0).is_some());
        assert!(region.ptr_at(4095).is_some());
        assert!(region.ptr_at(4096).is_none());
    }

    #[test]
    fn irq_line_gsi() {
        // SAFETY: test-only, no real hardware.
        let irq = unsafe { IrqLine::new(10) };
        assert_eq!(irq.gsi(), 10);
    }
}
