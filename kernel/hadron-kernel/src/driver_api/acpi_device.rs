//! ACPI device information and matching types for platform driver discovery.
//!
//! When the kernel walks the AML namespace, devices with `_HID` values are
//! collected into [`AcpiDeviceInfo`] structs. Platform drivers declare which
//! ACPI hardware IDs they support via [`AcpiMatchId`] tables, and the kernel
//! matches devices to drivers by comparing `_HID` / `_CID` values.

extern crate alloc;

use alloc::vec::Vec;

use hadron_acpi::aml::path::AmlPath;
use hadron_acpi::aml::value::AmlValue;
use hadron_acpi::resource::AcpiResource;

/// Identifies an ACPI device by hardware ID for driver matching.
#[derive(Clone, Copy)]
pub enum AcpiDeviceId {
    /// Compressed EISA/PnP ID (e.g., PNP0501).
    Eisa(u32),
    /// String ID (static, for driver tables).
    String(&'static str),
}

/// Match table entry for platform driver ACPI ID matching.
#[derive(Clone, Copy)]
pub struct AcpiMatchId {
    /// The device ID to match against.
    pub id: AcpiDeviceId,
}

impl AcpiMatchId {
    /// Create a match entry from a 7-char EISA ID string (e.g., `"PNP0501"`).
    ///
    /// The string is encoded at const time into the 32-bit compressed EISA format.
    pub const fn eisa(id: &'static str) -> Self {
        let bytes = id.as_bytes();
        assert!(bytes.len() == 7, "EISA ID must be exactly 7 characters");

        // Encode manufacturer code (3 uppercase letters).
        let c1 = (bytes[0] - b'@') as u16;
        let c2 = (bytes[1] - b'@') as u16;
        let c3 = (bytes[2] - b'@') as u16;
        let manufacturer = (c1 << 10) | (c2 << 5) | c3;

        // Encode product ID (4 hex digits).
        let d0 = hex_digit(bytes[3]) as u16;
        let d1 = hex_digit(bytes[4]) as u16;
        let d2 = hex_digit(bytes[5]) as u16;
        let d3 = hex_digit(bytes[6]) as u16;
        let product = (d0 << 12) | (d1 << 8) | (d2 << 4) | d3;

        // Combine into the byte-swapped format used in AML.
        // Native (big-endian) layout: manufacturer in upper 16, product in lower 16.
        let native = ((manufacturer as u32) << 16) | (product as u32);
        let raw = native.swap_bytes();

        Self {
            id: AcpiDeviceId::Eisa(raw),
        }
    }

    /// Create a match entry from a string ID (e.g., `"QEMU0002"`).
    pub const fn string(id: &'static str) -> Self {
        Self {
            id: AcpiDeviceId::String(id),
        }
    }

    /// Check if this match entry matches a device's `_HID` or `_CID`.
    ///
    /// ACPI `_HID` can be either a compressed EISA integer or a Buffer-encoded
    /// EISA ID — both use the same raw u32 representation.
    pub fn matches_hid(&self, hid: &AmlValue) -> bool {
        match (&self.id, hid) {
            (AcpiDeviceId::Eisa(raw), AmlValue::EisaId(id)) => *raw == id.raw,
            (AcpiDeviceId::Eisa(raw), AmlValue::Integer(v)) => *raw == *v as u32,
            (AcpiDeviceId::String(s), AmlValue::String(is)) => *s == is.as_str(),
            _ => false,
        }
    }
}

/// Decode a hex digit at const time.
const fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'A'..=b'F' => b - b'A' + 10,
        b'a'..=b'f' => b - b'a' + 10,
        _ => panic!("invalid hex digit in EISA ID"),
    }
}

/// Information about an ACPI-discovered platform device.
#[derive(Clone)]
pub struct AcpiDeviceInfo {
    /// Full ACPI namespace path (e.g., `\_SB_.PCI0.SF8_.COM1`).
    pub path: AmlPath,
    /// Primary hardware ID (`_HID`).
    pub hid: AmlValue,
    /// Compatible ID (`_CID`), for fallback matching.
    pub cid: Option<AmlValue>,
    /// Unique ID (`_UID`), for multi-instance devices.
    pub uid: Option<u64>,
    /// Decoded resources from `_CRS`.
    pub resources: Vec<AcpiResource>,
}

impl AcpiDeviceInfo {
    /// Returns the first I/O port resource, if any.
    ///
    /// Returns `(base, length)`.
    pub fn io_port(&self) -> Option<(u16, u16)> {
        for r in &self.resources {
            match r {
                AcpiResource::Io { base, length } => return Some((*base, *length)),
                AcpiResource::FixedIo { base, length } => {
                    return Some((*base, *length as u16));
                }
                _ => {}
            }
        }
        None
    }

    /// Returns the first IRQ resource, if any.
    pub fn irq(&self) -> Option<u8> {
        for r in &self.resources {
            match r {
                AcpiResource::Irq { irq, .. } => return Some(*irq),
                AcpiResource::ExtendedIrq { gsi, .. } => {
                    return Some(*gsi as u8);
                }
                _ => {}
            }
        }
        None
    }

    /// Returns the first memory region, if any.
    ///
    /// Returns `(base, length)`.
    pub fn memory_region(&self) -> Option<(u64, u64)> {
        for r in &self.resources {
            match r {
                AcpiResource::Memory32 { base, length, .. } => {
                    return Some((u64::from(*base), u64::from(*length)));
                }
                AcpiResource::FixedMemory32 { base, length, .. } => {
                    return Some((u64::from(*base), u64::from(*length)));
                }
                AcpiResource::Memory64 { base, length, .. } => {
                    return Some((*base, *length));
                }
                _ => {}
            }
        }
        None
    }
}
