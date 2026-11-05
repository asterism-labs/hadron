use core::marker::PhantomData;

use crate::protocol::file::{
    FileAttributes, FileInfo, FileMode, FileProtocol, SimpleFileSystemProtocol,
};
use crate::{EfiGuid, EfiStatus};

use super::utf8_to_ucs2;

/// Safe wrapper around the UEFI Simple File System Protocol.
pub struct FileSystem<'st> {
    raw: *mut SimpleFileSystemProtocol,
    _lifetime: PhantomData<&'st ()>,
}

impl<'st> FileSystem<'st> {
    /// Create a new `FileSystem` wrapper from a protocol reference obtained via `locate_protocol`.
    pub fn new(raw: &'st mut SimpleFileSystemProtocol) -> Self {
        Self {
            raw: core::ptr::from_mut(raw),
            _lifetime: PhantomData,
        }
    }

    /// Open the root directory of this file system volume.
    ///
    /// # Errors
    ///
    /// Returns the status from `OpenVolume` if it fails, for example
    /// `EFI_NO_MEDIA` or `EFI_DEVICE_ERROR`.
    pub fn open_volume(&self) -> Result<File<'st>, EfiStatus> {
        let mut root: *mut FileProtocol = core::ptr::null_mut();
        let status = unsafe { ((*self.raw).open_volume)(self.raw, &raw mut root) };
        status.to_result()?;
        Ok(File {
            raw: root,
            _lifetime: PhantomData,
        })
    }
}

/// Safe RAII wrapper around a UEFI file handle.
///
/// Automatically calls `close` on drop.
pub struct File<'st> {
    raw: *mut FileProtocol,
    _lifetime: PhantomData<&'st ()>,
}

impl<'st> File<'st> {
    /// Open a file or directory relative to this directory handle.
    ///
    /// `name` is a UTF-8 path (converted to UCS-2 internally; max ~255 characters).
    ///
    /// # Errors
    ///
    /// Returns `EFI_BUFFER_TOO_SMALL` if `name` has more than 255 characters.
    /// Otherwise returns the status from `Open` if it fails, for example
    /// `EFI_NOT_FOUND` when the file does not exist.
    pub fn open(
        &self,
        name: &str,
        mode: FileMode,
        attributes: FileAttributes,
    ) -> Result<File<'st>, EfiStatus> {
        let mut name_buf = [0u16; 256];
        utf8_to_ucs2(name, &mut name_buf)?;

        let mut new_handle: *mut FileProtocol = core::ptr::null_mut();
        let status = unsafe {
            ((*self.raw).open)(
                self.raw,
                &raw mut new_handle,
                name_buf.as_ptr(),
                mode.bits(),
                attributes.bits(),
            )
        };
        status.to_result()?;
        Ok(File {
            raw: new_handle,
            _lifetime: PhantomData,
        })
    }

    /// Read from the file into the buffer.
    ///
    /// Returns the number of bytes actually read. A return value of `0` indicates
    /// end-of-file.
    ///
    /// # Errors
    ///
    /// Returns the status from `Read` if it fails, for example
    /// `EFI_BUFFER_TOO_SMALL` when reading a directory entry that does not fit in `buf`.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, EfiStatus> {
        let mut size = buf.len();
        let status = unsafe { ((*self.raw).read)(self.raw, &raw mut size, buf.as_mut_ptr()) };
        status.to_result()?;
        Ok(size)
    }

    /// Get file information.
    ///
    /// The caller provides a buffer large enough for [`FileInfo`] plus the
    /// variable-length filename. 256 bytes is usually sufficient.
    ///
    /// # Errors
    ///
    /// Returns `EFI_INVALID_PARAMETER` if `buf` is not aligned for [`FileInfo`].
    /// Otherwise returns the status from `GetInfo` if it fails, for example
    /// `EFI_BUFFER_TOO_SMALL` when `buf` cannot hold the information.
    pub fn get_info<'buf>(&self, buf: &'buf mut [u8]) -> Result<&'buf FileInfo, EfiStatus> {
        if buf.as_ptr().addr() % core::mem::align_of::<FileInfo>() != 0 {
            return Err(EfiStatus::INVALID_PARAMETER);
        }
        let mut size = buf.len();
        let status = unsafe {
            ((*self.raw).get_info)(
                self.raw,
                core::ptr::from_ref::<EfiGuid>(&EfiGuid::FILE_INFO),
                &raw mut size,
                buf.as_mut_ptr(),
            )
        };
        status.to_result()?;
        // SAFETY: The firmware wrote a `FileInfo` at the start of `buf`, and the
        // alignment check above makes the cast valid.
        #[allow(clippy::cast_ptr_alignment)]
        let info = unsafe { &*buf.as_ptr().cast::<FileInfo>() };
        Ok(info)
    }

    /// Set the file position.
    ///
    /// Pass `0xFFFF_FFFF_FFFF_FFFF` to seek to end-of-file.
    ///
    /// # Errors
    ///
    /// Returns the status from `SetPosition` if it fails, for example
    /// `EFI_UNSUPPORTED` when seeking a directory to a nonzero position.
    pub fn set_position(&self, position: u64) -> Result<(), EfiStatus> {
        let status = unsafe { ((*self.raw).set_position)(self.raw, position) };
        status.to_result()
    }

    /// Convenience method: get the file size in bytes.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`get_info`](Self::get_info).
    pub fn file_size(&self, info_buf: &mut [u8]) -> Result<u64, EfiStatus> {
        let info = self.get_info(info_buf)?;
        Ok(info.file_size)
    }
}

impl Drop for File<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe { ((*self.raw).close)(self.raw) };
        }
    }
}
