use core::marker::PhantomData;

use crate::protocol::file::SimpleFileSystemProtocol;
use crate::protocol::gop::GraphicsOutputProtocol;
use crate::protocol::loaded_image::LoadedImageProtocol;
use crate::protocol::simple_text::SimpleTextOutputProtocol;
use crate::table;
use crate::{EfiGuid, EfiHandle, EfiStatus};

/// Boot services wrapper.
pub mod boot;
/// Console I/O wrapper with `fmt::Write` support.
pub mod console;
/// File system and file RAII wrappers.
pub mod fs;
/// Graphics Output Protocol wrapper.
pub mod gop;
/// Memory map iterator.
pub mod memory;

pub use boot::BootServices;
pub use console::Console;
pub use fs::{File, FileSystem};
pub use gop::Gop;
pub use memory::MemoryMap;

// ---------------------------------------------------------------------------
// Type-state markers
// ---------------------------------------------------------------------------

/// Marker type representing the boot-services-active phase.
pub enum Boot {}

/// Marker type representing the runtime (post-`ExitBootServices`) phase.
pub enum Runtime {}

mod sealed {
    pub trait Phase {}
    impl Phase for super::Boot {}
    impl Phase for super::Runtime {}
}

// ---------------------------------------------------------------------------
// Protocol trait
// ---------------------------------------------------------------------------

/// Trait for UEFI protocols that can be located via `BootServices::locate_protocol`.
///
/// # Safety
///
/// Implementors must provide the correct GUID and raw FFI type for the protocol.
pub unsafe trait Protocol {
    /// The protocol GUID used to locate this protocol.
    const GUID: EfiGuid;
    /// The raw FFI struct type for this protocol.
    type Raw;
}

/// Marker for [`SimpleTextOutputProtocol`].
pub enum SimpleTextOutputId {}
unsafe impl Protocol for SimpleTextOutputId {
    const GUID: EfiGuid = EfiGuid::SIMPLE_TEXT_OUTPUT_PROTOCOL;
    type Raw = SimpleTextOutputProtocol;
}

/// Marker for [`GraphicsOutputProtocol`].
pub enum GraphicsOutputId {}
unsafe impl Protocol for GraphicsOutputId {
    const GUID: EfiGuid = EfiGuid::GRAPHICS_OUTPUT_PROTOCOL;
    type Raw = GraphicsOutputProtocol;
}

/// Marker for [`SimpleFileSystemProtocol`].
pub enum SimpleFileSystemId {}
unsafe impl Protocol for SimpleFileSystemId {
    const GUID: EfiGuid = EfiGuid::SIMPLE_FILE_SYSTEM_PROTOCOL;
    type Raw = SimpleFileSystemProtocol;
}

/// Marker for [`LoadedImageProtocol`].
pub enum LoadedImageId {}
unsafe impl Protocol for LoadedImageId {
    const GUID: EfiGuid = EfiGuid::LOADED_IMAGE_PROTOCOL;
    type Raw = LoadedImageProtocol;
}

// ---------------------------------------------------------------------------
// UTF-8 to UCS-2 helper
// ---------------------------------------------------------------------------

/// Convert a UTF-8 string to a null-terminated UCS-2 string in the provided buffer.
///
/// Returns the number of `u16` units written **including** the null terminator.
/// Non-BMP characters (above U+FFFF) are replaced with U+FFFD.
pub(crate) fn utf8_to_ucs2(s: &str, buf: &mut [u16]) -> Result<usize, EfiStatus> {
    if buf.is_empty() {
        return Err(EfiStatus::BUFFER_TOO_SMALL);
    }
    let mut i = 0;
    for ch in s.chars() {
        let code = if (ch as u32) > 0xFFFF {
            0xFFFD // replacement character
        } else {
            ch as u16
        };
        // Need space for this character + null terminator
        if i + 1 >= buf.len() {
            return Err(EfiStatus::BUFFER_TOO_SMALL);
        }
        buf[i] = code;
        i += 1;
    }
    buf[i] = 0;
    Ok(i + 1)
}

// ---------------------------------------------------------------------------
// SystemTable
// ---------------------------------------------------------------------------

/// Safe wrapper around the UEFI System Table, parameterized by boot phase.
///
/// `SystemTable<Boot>` provides access to boot services, console I/O, and protocols.
/// Calling [`exit_boot_services`](SystemTable::exit_boot_services) consumes the
/// `Boot` table and returns a `SystemTable<Runtime>`, preventing further use of
/// boot-time resources at compile time.
pub struct SystemTable<S: sealed::Phase> {
    handle: EfiHandle,
    raw: *mut table::SystemTable,
    _phase: PhantomData<S>,
}

impl SystemTable<Boot> {
    /// Create a `SystemTable<Boot>` from the raw pointers passed to `efi_main`.
    ///
    /// # Safety
    ///
    /// - `handle` must be a valid EFI image handle.
    /// - `raw` must point to a valid, firmware-owned UEFI System Table.
    /// - This must only be called once per boot.
    pub unsafe fn from_raw(handle: EfiHandle, raw: *mut table::SystemTable) -> Self {
        Self {
            handle,
            raw,
            _phase: PhantomData,
        }
    }

    /// Returns the image handle.
    pub fn image_handle(&self) -> EfiHandle {
        self.handle
    }

    /// Borrow the boot services table.
    pub fn boot_services(&self) -> BootServices<'_> {
        let bs = unsafe { &*(*self.raw).boot_services };
        BootServices::new(bs, self.handle)
    }

    /// Get a console wrapper for standard output.
    pub fn console_out(&self) -> Console<'_> {
        let raw = unsafe { (*self.raw).console_out as *mut SimpleTextOutputProtocol };
        Console::new(raw)
    }

    /// Get a console wrapper for standard error.
    pub fn console_err(&self) -> Console<'_> {
        let raw = unsafe { (*self.raw).standard_error as *mut SimpleTextOutputProtocol };
        Console::new(raw)
    }

    /// Exit boot services, transitioning to the runtime phase.
    ///
    /// This **consumes** `self`, so any outstanding borrows of boot-time resources
    /// (`BootServices`, `Console`, `Gop`, `File`, etc.) will cause a compile error.
    ///
    /// The caller must provide a buffer for the memory map (8 KiB is a safe default).
    /// On success, returns the runtime system table and the final memory map.
    pub fn exit_boot_services(
        self,
        buf: &mut [u8],
    ) -> Result<(SystemTable<Runtime>, MemoryMap<'_>), EfiStatus> {
        let bs = unsafe { &*(*self.raw).boot_services };

        // First attempt: get memory map, then exit
        let mut map_size = buf.len();
        let mut map_key: usize = 0;
        let mut desc_size: usize = 0;
        let mut desc_version: u32 = 0;

        let status = unsafe {
            (bs.get_memory_map)(
                &mut map_size,
                buf.as_mut_ptr(),
                &mut map_key,
                &mut desc_size,
                &mut desc_version,
            )
        };
        status.to_result()?;

        let status = unsafe { (bs.exit_boot_services)(self.handle, map_key) };

        if status.is_success() {
            let rt = SystemTable::<Runtime> {
                handle: self.handle,
                raw: self.raw,
                _phase: PhantomData,
            };
            let map = MemoryMap::new(&buf[..map_size], map_key, desc_size, desc_version);
            return Ok((rt, map));
        }

        // Retry once if the map key was stale (firmware may have changed the map
        // between get_memory_map and exit_boot_services).
        if status == EfiStatus::INVALID_PARAMETER {
            map_size = buf.len();
            let status = unsafe {
                (bs.get_memory_map)(
                    &mut map_size,
                    buf.as_mut_ptr(),
                    &mut map_key,
                    &mut desc_size,
                    &mut desc_version,
                )
            };
            status.to_result()?;

            let status = unsafe { (bs.exit_boot_services)(self.handle, map_key) };
            status.to_result()?;

            let rt = SystemTable::<Runtime> {
                handle: self.handle,
                raw: self.raw,
                _phase: PhantomData,
            };
            let map = MemoryMap::new(&buf[..map_size], map_key, desc_size, desc_version);
            return Ok((rt, map));
        }

        Err(status)
    }
}

impl SystemTable<Runtime> {
    /// Returns a reference to the runtime services table.
    pub fn runtime_services(&self) -> &table::RuntimeServices {
        unsafe { &*(*self.raw).runtime_services }
    }

    /// Returns the UEFI configuration tables.
    pub fn configuration_tables(&self) -> &[table::ConfigurationTable] {
        unsafe {
            let st = &*self.raw;
            core::slice::from_raw_parts(st.configuration_table, st.number_of_table_entries)
        }
    }
}
