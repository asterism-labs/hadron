//! UEFI Loaded Image Protocol.
//!
//! The Loaded Image Protocol provides information about a loaded UEFI image (application
//! or driver). It is installed on every image handle.

use core::ffi::c_void;

use crate::{EfiHandle, EfiStatus, memory::EfiMemoryType, table::SystemTable};

use super::device_path::DevicePathProtocol;

/// The Loaded Image Protocol.
///
/// Contains information about a loaded UEFI image, including its memory location,
/// load options, device handle, and file path.
#[repr(C)]
pub struct LoadedImageProtocol {
    /// The revision of this protocol.
    pub revision: u32,
    /// The image's handle (the same handle that this protocol is installed on).
    pub parent_handle: EfiHandle,
    /// Pointer to the System Table.
    pub system_table: *mut SystemTable,

    // ── Source location ──────────────────────────────────────────
    /// The device handle from which the image was loaded.
    pub device_handle: EfiHandle,
    /// Pointer to the device path of the file from which the image was loaded.
    pub file_path: *mut DevicePathProtocol,
    /// Reserved field.
    pub reserved: *mut c_void,

    // ── Load options ─────────────────────────────────────────────
    /// The size in bytes of `load_options`.
    pub load_options_size: u32,
    /// Pointer to the image's load options (typically a UCS-2 command line string).
    pub load_options: *mut c_void,

    // ── Location of the image in memory ──────────────────────────
    /// Pointer to the base address of the loaded image in memory.
    pub image_base: *mut c_void,
    /// The size in bytes of the loaded image.
    pub image_size: u64,
    /// The memory type that the code sections were loaded as.
    pub image_code_type: EfiMemoryType,
    /// The memory type that the data sections were loaded as.
    pub image_data_type: EfiMemoryType,
    /// The `Unload` function for this image.
    pub unload: unsafe extern "efiapi" fn(image_handle: EfiHandle) -> EfiStatus,
}

// ── Compile-time layout assertions ──────────────────────────────────

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<LoadedImageProtocol>() == 96);
    // Padding after revision (u32) before parent_handle (ptr)
    assert!(core::mem::offset_of!(LoadedImageProtocol, revision) == 0);
    assert!(core::mem::offset_of!(LoadedImageProtocol, parent_handle) == 8);
    assert!(core::mem::offset_of!(LoadedImageProtocol, system_table) == 16);
    assert!(core::mem::offset_of!(LoadedImageProtocol, device_handle) == 24);
    assert!(core::mem::offset_of!(LoadedImageProtocol, file_path) == 32);
    assert!(core::mem::offset_of!(LoadedImageProtocol, reserved) == 40);
    // Padding after load_options_size (u32) before load_options (ptr)
    assert!(core::mem::offset_of!(LoadedImageProtocol, load_options_size) == 48);
    assert!(core::mem::offset_of!(LoadedImageProtocol, load_options) == 56);
    assert!(core::mem::offset_of!(LoadedImageProtocol, image_base) == 64);
    assert!(core::mem::offset_of!(LoadedImageProtocol, image_size) == 72);
    assert!(core::mem::offset_of!(LoadedImageProtocol, image_code_type) == 80);
    assert!(core::mem::offset_of!(LoadedImageProtocol, image_data_type) == 84);
    assert!(core::mem::offset_of!(LoadedImageProtocol, unload) == 88);
};
