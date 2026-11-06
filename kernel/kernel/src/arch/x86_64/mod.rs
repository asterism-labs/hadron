//! `x86_64` support: port I/O, CPU control, context switches, the GDT, the
//! IDT, exception and interrupt entry, the local APIC and its timer, the
//! legacy PIC and PIT, the HPET, the TSC, and the QEMU exit device.

pub(crate) mod context;
pub(crate) mod cpu;
pub(crate) mod gdt;
pub(crate) mod hpet;
pub(crate) mod idt;
pub(crate) mod irq;
pub(crate) mod lapic;
pub(crate) mod pic;
pub(crate) mod pit;
pub(crate) mod port;
pub(crate) mod qemu;
pub(crate) mod trap;
pub(crate) mod tsc;
