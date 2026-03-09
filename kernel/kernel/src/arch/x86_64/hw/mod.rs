//! Arch-critical hardware drivers (APIC, PIC, timers).
//!
//! These are kernel infrastructure, not pluggable drivers. Analogous to
//! `arch/x86/kernel/` in Linux — APIC, HPET, PIT, and PCI core live here
//! because the kernel's interrupt and timer subsystems depend on them directly.

#[cfg(hadron_hpet)]
pub mod hpet;
#[cfg(hadron_apic)]
pub mod io_apic;
#[cfg(hadron_apic)]
pub mod local_apic;
pub mod pic;
pub mod pit;
pub mod rtc;
pub mod tsc;
