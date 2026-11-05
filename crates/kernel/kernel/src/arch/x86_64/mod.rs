//! `x86_64` support: port I/O, CPU control, the GDT, the IDT and exception
//! entry, and the QEMU exit device.

pub(crate) mod cpu;
pub(crate) mod gdt;
pub(crate) mod idt;
pub(crate) mod port;
pub(crate) mod qemu;
pub(crate) mod tables;
pub(crate) mod trap;
