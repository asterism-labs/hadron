//! PCI bus access for x86_64.

#[cfg(target_arch = "x86_64")]
pub mod cam;
#[cfg(target_arch = "x86_64")]
pub mod caps;
#[cfg(target_arch = "x86_64")]
pub mod enumerate;
#[cfg(target_arch = "x86_64")]
pub mod msix;
#[cfg(target_arch = "x86_64")]
pub mod stub;
