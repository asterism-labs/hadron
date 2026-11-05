//! `x86_64` support: port I/O, CPU control, the GDT and the QEMU exit
//! device.

pub(crate) mod cpu;
pub(crate) mod gdt;
pub(crate) mod port;
pub(crate) mod qemu;
pub(crate) mod tables;
