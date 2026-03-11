//! x86_64 CPU instruction and register wrappers.
//!
//! This crate provides typed, safe-where-possible wrappers around x86_64
//! inline assembly for control registers, MSRs, segment operations, port I/O,
//! TLB management, and other CPU instructions. It centralizes all inline
//! assembly into a single crate so the rest of the kernel can avoid raw `asm!`
//! blocks.
//!
//! # Safety Classification
//!
//! Operations that have no side effects (reading CR0, RFLAGS, segment
//! registers, CPUID, RDTSC) are exposed as safe functions. Operations that
//! change CPU state, perform I/O, or require specific preconditions are
//! marked `unsafe` with documented safety contracts.

#![no_std]
#![warn(missing_docs)]

pub use hadron_core::addr::{PhysAddr, VirtAddr};

pub mod cpuid;
pub mod instructions;
pub mod registers;
pub mod structures;
