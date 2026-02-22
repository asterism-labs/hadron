//! Rust bindings for the UEFI specification.
//!
//! This crate provides type-safe Rust bindings for the [Unified Extensible Firmware Interface
//! (UEFI)](https://uefi.org/) specification, enabling direct interaction with UEFI firmware
//! services from Rust. Used internally by Hadron OS.
//!
//! # Overview
//!
//! UEFI is the modern firmware interface that replaces the legacy BIOS. This crate exposes both
//! raw `#[repr(C)]` FFI types that match the UEFI specification layout, and safe wrapper methods
//! for common operations.
//!
//! # Architecture
//!
//! The crate is organized into several modules:
//! - [`status`] - Status code type with all UEFI error and warning codes
//! - [`guid`] - GUID type and well-known GUID constants
//! - [`memory`] - Memory types, descriptors, and attribute flags
//! - [`table`] - System table, boot services, and runtime services
//! - [`protocol`] - UEFI protocol definitions (GOP, text I/O, file system, etc.)
//!
//! # Calling Convention
//!
//! All UEFI function pointers use the `extern "efiapi"` calling convention, which maps to the
//! platform's native UEFI calling convention (MS x64 on x86-64, AAPCS on ARM).
//!
//! # Safety
//!
//! Most types in this crate are raw FFI types. Calling UEFI functions through the function
//! pointers requires `unsafe` code and adherence to the UEFI specification's requirements.
//! Safe wrapper methods are provided where practical and are documented with their safety
//! requirements.
//!
//! ## `bool` in FFI
//!
//! UEFI's `BOOLEAN` type is a `UINT8` with values `TRUE` (1) and `FALSE` (0).
//! This crate uses Rust's `bool` directly in `extern "efiapi"` function signatures
//! and struct fields. This is valid because compliant UEFI firmware always passes
//! 0 or 1 for boolean values, matching Rust's `bool` validity invariant.

#![no_std]
#![feature(c_variadic)]

/// Safe, high-level wrappers for UEFI services using type-state and RAII patterns.
pub mod api;
pub mod guid;
pub mod memory;
pub mod protocol;
pub mod status;
pub mod table;

use core::ffi::c_void;

pub use guid::EfiGuid;
pub use status::EfiStatus;

/// An opaque handle to a UEFI object (protocol, image, device, etc.).
pub type EfiHandle = *mut c_void;

/// An opaque handle to a UEFI event.
pub type EfiEvent = *mut c_void;

/// A physical memory address.
pub type EfiPhysicalAddress = u64;

/// A virtual memory address.
pub type EfiVirtualAddress = u64;

/// A task priority level.
pub type EfiTpl = usize;

/// UEFI Task Priority Level constants.
pub mod tpl {
    use super::EfiTpl;

    /// Application level (lowest priority).
    pub const APPLICATION: EfiTpl = 4;
    /// Callback level.
    pub const CALLBACK: EfiTpl = 8;
    /// Notify level.
    pub const NOTIFY: EfiTpl = 16;
    /// High level (highest priority, masks all interrupts).
    pub const HIGH_LEVEL: EfiTpl = 31;
}
