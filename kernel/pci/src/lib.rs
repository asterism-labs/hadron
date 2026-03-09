//! PCI bus enumeration and capability parsing for Hadron OS.
//!
//! This crate contains the portable PCI logic: register constants, device
//! enumeration algorithm, capability linked-list walking, VirtIO/MSI-X
//! capability parsing, and class name lookup.
//!
//! Hardware access is abstracted behind the [`PciConfigAccess`] trait.
//! The kernel provides concrete implementations (legacy CAM via I/O ports,
//! ECAM via MMIO) in `hadron-kernel`.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

extern crate alloc;

pub mod caps;
pub mod enumerate;
pub mod regs;

/// Trait for reading/writing PCI configuration space.
///
/// Concrete implementations live in the kernel (CAM via I/O ports, ECAM via MMIO).
/// Tests can provide a mock implementation backed by in-memory arrays.
///
/// # Safety
///
/// Implementations must ensure that BDF addresses and register offsets are
/// valid for the underlying hardware access mechanism.
pub trait PciConfigAccess {
    /// Reads a 32-bit value from PCI config space.
    ///
    /// # Safety
    ///
    /// The caller must ensure the BDF address refers to a valid PCI device.
    unsafe fn read_u32(&self, bus: u8, device: u8, function: u8, offset: u8) -> u32;

    /// Reads a 16-bit value from PCI config space.
    ///
    /// # Safety
    ///
    /// Same requirements as [`read_u32`](Self::read_u32).
    unsafe fn read_u16(&self, bus: u8, device: u8, function: u8, offset: u8) -> u16 {
        let dword = unsafe { self.read_u32(bus, device, function, offset) };
        let shift = ((offset & 2) as u32) * 8;
        (dword >> shift) as u16
    }

    /// Reads an 8-bit value from PCI config space.
    ///
    /// # Safety
    ///
    /// Same requirements as [`read_u32`](Self::read_u32).
    unsafe fn read_u8(&self, bus: u8, device: u8, function: u8, offset: u8) -> u8 {
        let dword = unsafe { self.read_u32(bus, device, function, offset) };
        let shift = ((offset & 3) as u32) * 8;
        (dword >> shift) as u8
    }

    /// Writes a 32-bit value to PCI config space.
    ///
    /// # Safety
    ///
    /// PCI config space writes can have side effects on hardware.
    unsafe fn write_u32(&self, bus: u8, device: u8, function: u8, offset: u8, val: u32);
}

/// Returns a human-readable name for a PCI class/subclass pair.
#[must_use]
pub fn class_name(class: u8, subclass: u8) -> &'static str {
    match (class, subclass) {
        (0x00, 0x00) => "Non-VGA Unclassified",
        (0x01, 0x01) => "IDE Controller",
        (0x01, 0x06) => "SATA Controller",
        (0x02, 0x00) => "Ethernet Controller",
        (0x03, 0x00) => "VGA Controller",
        (0x04, 0x00) => "Video Device",
        (0x06, 0x00) => "Host Bridge",
        (0x06, 0x01) => "ISA Bridge",
        (0x06, 0x04) => "PCI-to-PCI Bridge",
        (0x08, 0x00) => "PIC",
        (0x08, 0x03) => "RTC Controller",
        (0x0C, 0x03) => "USB Controller",
        (0x0C, 0x05) => "SMBus Controller",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests;
