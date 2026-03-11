//! Shared syscall interface definitions for the Hadron kernel and userspace.
//!
//! This crate provides syscall numbers, error codes, `#[repr(C)]` types, and
//! flag constants shared between the kernel-side dispatch and the userspace
//! syscall wrappers. The [`wrappers`] module contains inline-assembly syscall
//! stubs that are only compiled for the `hadron` target OS.

#![no_std]

pub mod constants;
pub mod errors;
pub mod numbers;
pub mod types;
/// Inline-assembly syscall stubs for userspace.
///
/// Only available on x86_64 targets (the `syscall` instruction is
/// x86_64-specific). Kernel code should not call these — they are
/// for userspace binaries only.
#[cfg(target_arch = "x86_64")]
pub mod wrappers;

pub use constants::*;
pub use errors::*;
pub use numbers::*;
pub use types::*;
