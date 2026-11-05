//! `x86_64` support: port I/O, CPU control, context switches, the GDT, the
//! IDT, exception and interrupt entry, the legacy PIC, and the QEMU exit
//! device.

pub(crate) mod context;
pub(crate) mod cpu;
pub(crate) mod gdt;
pub(crate) mod idt;
pub(crate) mod irq;
pub(crate) mod pic;
pub(crate) mod port;
pub(crate) mod qemu;
pub(crate) mod tables;
pub(crate) mod trap;
