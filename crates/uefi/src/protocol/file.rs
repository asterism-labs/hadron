//! UEFI Simple File System and File protocols.
//!
//! The Simple File System Protocol provides access to a FAT file system volume.
//! The File Protocol provides an interface for file I/O operations.

use bitflags::bitflags;

use crate::{EfiGuid, EfiStatus};

/// The Simple File System Protocol.
///
/// This protocol is used to obtain a handle to a file system's root directory.
#[repr(C)]
pub struct SimpleFileSystemProtocol {
    /// The revision of this protocol.
    pub revision: u64,
    /// Opens the root directory of a volume.
    pub open_volume: unsafe extern "efiapi" fn(
        this: *mut SimpleFileSystemProtocol,
        root: *mut *mut FileProtocol,
    ) -> EfiStatus,
}

/// The File Protocol.
///
/// Provides file I/O access to a file system. Obtained from `SimpleFileSystemProtocol::open_volume`
/// or `FileProtocol::open`.
#[repr(C)]
pub struct FileProtocol {
    /// The revision of this protocol.
    pub revision: u64,
    /// Opens a new file relative to the source file's location.
    pub open: unsafe extern "efiapi" fn(
        this: *mut FileProtocol,
        new_handle: *mut *mut FileProtocol,
        file_name: *const u16,
        open_mode: u64,
        attributes: u64,
    ) -> EfiStatus,
    /// Closes the file handle.
    pub close: unsafe extern "efiapi" fn(this: *mut FileProtocol) -> EfiStatus,
    /// Closes and deletes the file.
    pub delete: unsafe extern "efiapi" fn(this: *mut FileProtocol) -> EfiStatus,
    /// Reads data from the file.
    pub read: unsafe extern "efiapi" fn(
        this: *mut FileProtocol,
        buffer_size: *mut usize,
        buffer: *mut u8,
    ) -> EfiStatus,
    /// Writes data to the file.
    pub write: unsafe extern "efiapi" fn(
        this: *mut FileProtocol,
        buffer_size: *mut usize,
        buffer: *const u8,
    ) -> EfiStatus,
    /// Returns the current file position.
    pub get_position:
        unsafe extern "efiapi" fn(this: *mut FileProtocol, position: *mut u64) -> EfiStatus,
    /// Sets the current file position.
    pub set_position:
        unsafe extern "efiapi" fn(this: *mut FileProtocol, position: u64) -> EfiStatus,
    /// Returns information about a file.
    pub get_info: unsafe extern "efiapi" fn(
        this: *mut FileProtocol,
        information_type: *const EfiGuid,
        buffer_size: *mut usize,
        buffer: *mut u8,
    ) -> EfiStatus,
    /// Sets information about a file.
    pub set_info: unsafe extern "efiapi" fn(
        this: *mut FileProtocol,
        information_type: *const EfiGuid,
        buffer_size: usize,
        buffer: *const u8,
    ) -> EfiStatus,
    /// Flushes all modified data associated with the file to the device.
    pub flush: unsafe extern "efiapi" fn(this: *mut FileProtocol) -> EfiStatus,
}

bitflags! {
    /// File open mode flags.
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileMode: u64 {
        /// Open for reading.
        const READ = 0x0000_0000_0000_0001;
        /// Open for writing.
        const WRITE = 0x0000_0000_0000_0002;
        /// Create the file if it does not exist.
        const CREATE = 0x8000_0000_0000_0000;
    }
}

bitflags! {
    /// File attribute flags.
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileAttributes: u64 {
        /// The file is read-only.
        const READ_ONLY = 0x0000_0000_0000_0001;
        /// The file is hidden.
        const HIDDEN = 0x0000_0000_0000_0002;
        /// The file is a system file.
        const SYSTEM = 0x0000_0000_0000_0004;
        /// Reserved (should not be used).
        const RESERVED = 0x0000_0000_0000_0008;
        /// The file is a directory.
        const DIRECTORY = 0x0000_0000_0000_0010;
        /// The file has been modified since last backup.
        const ARCHIVE = 0x0000_0000_0000_0020;
    }
}

/// File information structure.
///
/// This is a variable-length structure. The `file_name` field contains the first
/// character of a null-terminated UCS-2 filename; the actual filename extends
/// beyond this struct.
#[repr(C)]
pub struct FileInfo {
    /// The size of this `FileInfo` structure including the variable-length filename.
    pub size: u64,
    /// The size of the file in bytes.
    pub file_size: u64,
    /// The amount of physical space consumed by the file on the volume.
    pub physical_size: u64,
    /// The time the file was created.
    pub create_time: crate::table::EfiTime,
    /// The time the file was last accessed.
    pub last_access_time: crate::table::EfiTime,
    /// The time the file was last modified.
    pub modification_time: crate::table::EfiTime,
    /// The file attributes.
    pub attribute: u64,
    /// The first character of the null-terminated UCS-2 file name.
    ///
    /// The actual file name extends beyond this field. Use the `size` field
    /// to determine the total structure size.
    pub file_name: [u16; 1],
}

// ── Compile-time layout assertions ──────────────────────────────────

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<SimpleFileSystemProtocol>() == 16);
    assert!(core::mem::size_of::<FileProtocol>() == 88);
};
