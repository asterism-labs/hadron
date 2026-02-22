//! PCI Express Enhanced Configuration Access Mechanism (ECAM).
//!
//! ECAM provides memory-mapped access to the full 4 KiB PCI configuration
//! space, replacing the legacy 256-byte CAM I/O port mechanism. The ECAM
//! base address is discovered from the ACPI MCFG table.

use crate::addr::{PhysAddr, VirtAddr};
use crate::mm::hhdm;

/// Read a 32-bit value from PCI config space via ECAM.
///
/// Returns `None` if ECAM is not available or the bus is out of range.
pub fn ecam_read_u32(bus: u8, device: u8, function: u8, offset: u8) -> Option<u32> {
    let addr = ecam_address(bus, device, function, offset)?;
    // SAFETY: The ECAM region is identity-mapped via HHDM and the address
    // was validated to be within the MCFG-described range.
    Some(unsafe { (addr.as_ptr::<u32>()).read_volatile() })
}

/// Read a 16-bit value from PCI config space via ECAM.
pub fn ecam_read_u16(bus: u8, device: u8, function: u8, offset: u8) -> Option<u16> {
    let addr = ecam_address(bus, device, function, offset)?;
    Some(unsafe { (addr.as_ptr::<u16>()).read_volatile() })
}

/// Read an 8-bit value from PCI config space via ECAM.
pub fn ecam_read_u8(bus: u8, device: u8, function: u8, offset: u8) -> Option<u8> {
    let addr = ecam_address(bus, device, function, offset)?;
    Some(unsafe { (addr.as_ptr::<u8>()).read_volatile() })
}

/// Write a 32-bit value to PCI config space via ECAM.
pub fn ecam_write_u32(bus: u8, device: u8, function: u8, offset: u8, value: u32) -> bool {
    if let Some(addr) = ecam_address(bus, device, function, offset) {
        unsafe { (addr.as_ptr::<u32>() as *mut u32).write_volatile(value) };
        true
    } else {
        false
    }
}

/// Compute the ECAM virtual address for a given BDF + register offset.
///
/// ECAM address = base + (bus << 20) | (device << 15) | (function << 12) | offset
fn ecam_address(bus: u8, device: u8, function: u8, offset: u8) -> Option<VirtAddr> {
    crate::arch::x86_64::acpi::with_ecam(|info| {
        if bus < info.start_bus || bus > info.end_bus {
            return None;
        }
        let phys = info.phys_base
            + ((bus as u64) << 20)
            | ((device as u64) << 15)
            | ((function as u64) << 12)
            | ((offset as u64) & 0xFFC); // mask to 4-byte alignment for safety
        let virt = hhdm::phys_to_virt(PhysAddr::new(phys));
        Some(virt)
    })
    .flatten()
}
