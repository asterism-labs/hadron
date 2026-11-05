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
//! - [`InternalModuleFlags::COMPRESSED`] - Module is GZ-compressed and the bootloader
//!   decompresses it (honoured from module response revision 2)
//!
//! Internal modules need request revision 1, which `ModuleRequest::new` uses. The
//! bootloader honours them if the module response revision is 1 or later.
//!
//! # Example
//!
//! ```no_run
//! use limine::{ModuleRequest, module::{InternalModule, InternalModuleFlags}};
//! use core::ffi::c_char;
//!
//! static MODULE_PATH: &[u8] = b"driver.sys\0";
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
    /// Flags for `InternalModule`.
    #[repr(transparent)]
    pub struct InternalModuleFlags: u64 {
        /// Fail to boot if the module is not found.
        const REQUIRED = 1 << 0;
        /// The module is GZ-compressed, and the bootloader decompresses it. Honoured if
        /// the module response revision is 2 or later.
        const COMPRESSED = 1 << 1;
    }
}

/// A module that the executable asks the bootloader to load.
#[repr(C)]
pub struct InternalModule {
    /// Non-null path to the module, relative to the executable's location. It may end in
    /// `#` and a 128-character hexadecimal blake2b hash, which the bootloader checks.
    /// Under Secure Boot the hash is required.
    pub path: *const c_char,
    /// Non-null string associated with the module. Use an empty string if none is needed.
    pub string: *const c_char,
    /// Flags that change how the module is loaded.
    pub flags: InternalModuleFlags,
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    #[test]
    fn internal_module_matches_spec() {
        assert_eq!(InternalModuleFlags::REQUIRED.bits(), 1);
        assert_eq!(InternalModuleFlags::COMPRESSED.bits(), 2);
        assert_eq!(offset_of!(InternalModule, string), 8);
        assert_eq!(offset_of!(InternalModule, flags), 16);
        assert_eq!(size_of::<InternalModule>(), 24);
    }
}
