//! Structured device path names.
//!
//! Replaces ad-hoc string keys like `"ahci-0"` with hierarchical device paths
//! that encode the device's position in the hardware topology.

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A hierarchical device path (e.g., `pci/0000:00:1f.2/ahci/ahci-0`).
///
/// Device paths provide structured naming for devices in the registry,
/// replacing ad-hoc string keys.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DevicePath {
    segments: Vec<String>,
}

impl DevicePath {
    /// Creates a device path for a PCI device.
    ///
    /// Format: `pci/<bus>:<device>.<function>/<driver>/<driver>-<index>`
    pub fn pci(bus: u8, device: u8, function: u8, driver: &str, index: usize) -> Self {
        Self {
            segments: alloc::vec![
                String::from("pci"),
                format!("{:04x}:{:02x}:{:02x}.{}", 0, bus, device, function),
                String::from(driver),
                format!("{}-{}", driver, index),
            ],
        }
    }

    /// Creates a device path for a platform device.
    ///
    /// Format: `platform/<name>`
    pub fn platform(name: &str) -> Self {
        Self {
            segments: alloc::vec![String::from("platform"), String::from(name)],
        }
    }

    /// Returns the leaf (last) segment of the path.
    ///
    /// This is the backward-compatible device name (e.g., `"ahci-0"`).
    #[must_use]
    pub fn leaf(&self) -> &str {
        self.segments
            .last()
            .map(String::as_str)
            .unwrap_or("")
    }

    /// Returns the full path as a `/`-separated string.
    #[must_use]
    pub fn as_str(&self) -> String {
        self.segments.join("/")
    }

    /// Returns the path segments.
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

impl core::fmt::Display for DevicePath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut first = true;
        for seg in &self.segments {
            if !first {
                f.write_str("/")?;
            }
            f.write_str(seg)?;
            first = false;
        }
        Ok(())
    }
}
