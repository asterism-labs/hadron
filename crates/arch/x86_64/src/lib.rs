//! `x86_64` system structures as plain data: segment selectors, segment and
//! system descriptors, the GDT and LDT, the 64-bit TSS, the IDT and the CPU
//! exceptions.
//!
//! The crate has no inline assembly. The kernel's `arch::x86_64` module
//! loads these structures with `lgdt`, `lidt`, `ltr` and `lldt`, and the host test
//! suite checks every encoding.
//!
//! References are to the Intel SDM, Volume 3A: §3.4.2 (segment selectors),
//! §3.4.5 (segment descriptors), §3.5 (system descriptor types) and the
//! chapter on task management for the 64-bit TSS and its descriptor, §6.3
//! (exceptions) and §6.14 (64-bit interrupt handling).

#![cfg_attr(not(test), no_std)]

mod descriptor;
mod exception;
mod idt;
mod segment;
mod table;
mod tss;

pub use descriptor::{SegmentDescriptor, SystemDescriptor, SystemType};
pub use exception::Exception;
pub use idt::{GateDescriptor, GateType, Idt, IstIndex, VECTORS};
pub use segment::{PrivilegeLevel, SegmentSelector, TableIndicator};
pub use table::{DescriptorTable, DescriptorTablePointer, Gdt, Global, Ldt, Local, TableKind};
pub use tss::TaskStateSegment;
