//! Timer and clock source drivers.

#[cfg(target_arch = "x86_64")]
pub mod hpet;
#[cfg(target_arch = "x86_64")]
pub mod pit;
#[cfg(target_arch = "x86_64")]
pub mod tsc;
