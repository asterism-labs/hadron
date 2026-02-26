//! Serial port drivers.

#[cfg(target_arch = "x86_64")]
pub mod serial_async;
#[cfg(target_arch = "x86_64")]
pub mod uart16550;
