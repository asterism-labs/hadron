//! `x86_64` CPU support for Hadron.
//!
//! - [`structures`]: system structures as plain data. Segment selectors,
//!   segment and system descriptors, the GDT and LDT, the 64-bit TSS, the
//!   IDT, the CPU exceptions and 4-level page tables.
//! - [`registers`]: the control registers and the segment registers, with
//!   typed flags.
//! - [`instructions`]: the instructions that load and store the descriptor
//!   table registers and the task register.
//!
//! Physical and virtual addresses sit at the crate root.
//!
//! The inline assembly in `registers` and `instructions` builds only for
//! `x86_64`. Everything else, flag types included, builds on any host, and
//! the host test suite checks every encoding.

#![cfg_attr(not(test), no_std)]

mod addr;
pub mod instructions;
pub mod registers;
pub mod structures;

pub use addr::{PHYS_ADDR_BITS, PhysAddr, VIRT_ADDR_BITS, VirtAddr};
