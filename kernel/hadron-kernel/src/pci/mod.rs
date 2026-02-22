//! PCI bus core: configuration access, enumeration, and capability parsing.
//!
//! This is kernel infrastructure — PCI bus management lives in the kernel,
//! not in the pluggable driver crate. Individual PCI device drivers (MSI-X
//! setup, VirtIO transport, AHCI) remain in `hadron-drivers`.

#[cfg(target_arch = "x86_64")]
pub mod cam;
#[cfg(target_arch = "x86_64")]
pub mod caps;
#[cfg(target_arch = "x86_64")]
pub mod ecam;
#[cfg(target_arch = "x86_64")]
pub mod enumerate;
