//! Module representation.
//!
//! This module defines types for specifying and working with kernel modules.
//! Modules are additional files (drivers, configuration, etc.) that the bootloader
//! loads alongside the kernel.
//!
//! # Overview
//!
//! The [`InternalModule`] structure allows the kernel to specify which additional
//! files should be loaded by the bootloader. Modules can be marked as required
//! (boot fails if not found) or optional.
//!
//! # Module Flags
//!
//! - [`InternalModuleFlags::REQUIRED`] - Module must be present or boot fails
//! - [`InternalModuleFlags::COMPRESSED`] - Module is compressed and should be decompressed
//!
//! # Example
//!
//! ```no_run
//! use limine::{ModuleRequest, module::{InternalModule, InternalModuleFlags}};
//! use core::ffi::c_char;
//!
//! static MODULE_PATH: &[u8] = b"boot:///driver.sys\0";
//! static MODULE_STRING: &[u8] = b"Network Driver\0";
//!
//! static INTERNAL_MODULES: [&InternalModule; 1] = [&InternalModule {
//!     path: MODULE_PATH.as_ptr() as *const c_char,
//!     string: MODULE_STRING.as_ptr() as *const c_char,
//!     flags: InternalModuleFlags::REQUIRED,
//! }];
//!
//! static MODULE_PTRS: [*const InternalModule; 1] = [INTERNAL_MODULES[0]];
//!
//! #[used]
//! #[link_section = ".requests"]
//! static MODULE_REQUEST: ModuleRequest = ModuleRequest::new(
//!     MODULE_PTRS.as_ptr(),
//!     MODULE_PTRS.len() as u64,
//! );
//! ```

use core::ffi::c_char;

bitflags::bitflags! {
    /// Flags for InternalModule
    #[repr(C)]
    pub struct InternalModuleFlags: u64 {
        /// Indicates that the module is required
        const REQUIRED = 0x1;
        /// Indicates that the module is compressed
        const COMPRESSED = 0x2;
    }
}

/// Represents an internal module with its path, string representation, and flags
#[repr(C)]
pub struct InternalModule {
    /// Path to the module
    pub path: *const c_char,
    /// String representation of the module
    pub string: *const c_char,
    /// Flags associated with the module
    pub flags: InternalModuleFlags,
}
