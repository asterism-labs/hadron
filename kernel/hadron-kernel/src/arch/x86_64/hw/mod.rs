//! Arch-critical hardware drivers (APIC, PIC, timers).
//!
//! These are kernel infrastructure, not pluggable drivers. Analogous to
//! `arch/x86/kernel/` in Linux — APIC, HPET, PIT, and PCI core live here
//! because the kernel's interrupt and timer subsystems depend on them directly.

pub mod hpet;
pub mod io_apic;
pub mod local_apic;
pub mod pic;
pub mod pit;
pub mod tsc;
