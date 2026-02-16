//! Representation of files provided by the bootloader.
//!
//! This module provides types for working with files that the bootloader has loaded
//! into memory. These can include kernel modules, configuration files, or other
//! resources needed by the kernel.
//!
//! # Overview
//!
//! The [`File`] structure represents a file loaded by the bootloader and includes:
//! - The file's contents in memory
//! - File metadata (path, size, media type)
//! - Storage location information (partition, disk IDs, UUIDs)
//!
//! # Media Types
//!
//! Files can come from different sources:
//! - [`MediaType::Generic`] - Regular file from disk
//! - [`MediaType::Optical`] - File from optical media (CD/DVD)
//! - [`MediaType::Tftp`] - File loaded via TFTP network boot
//!
//! # UUID Types
//!
//! The module includes UUID support for identifying disks and partitions:
//! - GPT disk UUID - Identifies the entire disk
//! - GPT partition UUID - Identifies a specific partition
//! - Partition UUID - Generic partition identifier
//!
//! # Example
//!
//! ```no_run
//! use limine::ModuleRequest;
//!
//! #[used]
//! #[link_section = ".requests"]
//! static MODULE_REQUEST: ModuleRequest = ModuleRequest::new(
//!     core::ptr::null(),
//!     0,
//! );
//!
//! fn load_modules() {
//!     if let Some(module_response) = MODULE_REQUEST.response() {
//!         for file in module_response.modules() {
//!             println!("Loaded module: {}", file.path());
//!             println!("  Size: {} bytes", file.size);
//!             println!("  Address: {:p}", file.address);
//!
//!             // Access file contents
//!             let data = unsafe {
//!                 core::slice::from_raw_parts(
//!                     file.address as *const u8,
//!                     file.size as usize
//!                 )
//!             };
//!         }
//!     }
//! }
//! ```

use core::{
    ffi::{c_char, c_void},
    num::NonZeroU128,
    ptr::NonNull,
};

/// Media type of the file.
#[repr(u32)]
pub enum MediaType {
    /// A generic file from disk.
    Generic = 0,
    /// An optical media file (CD/DVD).
    Optical = 1,
    /// A file loaded via TFTP network boot.
    Tftp = 2,
}

/// Standard UUID structure with fields for time-based and node components.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Uuid {
    /// The low 32 bits of the time field.
    pub a: u32,
    /// The middle 16 bits of the time field.
    pub b: u16,
    /// The high 16 bits of the time field combined with the version.
    pub c: u16,
    /// The clock sequence and node (8 bytes).
    pub d: [u8; 8],
}

/// A non-zero UUID wrapper, guaranteed to be non-zero.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NonZeroUuid(NonZeroU128);

impl core::fmt::Debug for NonZeroUuid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get().fmt(f)
    }
}

impl NonZeroUuid {
    /// Converts this `NonZeroUuid` to a [`Uuid`].
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn get(&self) -> Uuid {
        let value = self.0.get();
        Uuid {
            a: (value >> 96) as u32,
            b: (value >> 80) as u16,
            c: (value >> 64) as u16,
            d: [
                (value >> 56) as u8,
                (value >> 48) as u8,
                (value >> 40) as u8,
                (value >> 32) as u8,
                (value >> 24) as u8,
                (value >> 16) as u8,
                (value >> 8) as u8,
                value as u8,
            ],
        }
    }
}

impl core::fmt::Debug for Uuid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.a,
            self.b,
            self.c,
            self.d[0],
            self.d[1],
            self.d[2],
            self.d[3],
            self.d[4],
            self.d[5],
            self.d[6],
            self.d[7]
        )
    }
}

/// Represents a file provided by the bootloader.
#[repr(C)]
pub struct File {
    /// Revision of the file structure.
    pub revision: u64,
    /// Address of the file in memory.
    pub address: *const c_void,
    /// Size of the file in bytes.
    pub size: u64,
    /// Path to the file as a C string.
    pub path: *const c_char,
    /// Additional string associated with the file as a C string.
    pub string: *const c_char,
    /// Media type of the file.
    pub media_type: MediaType,
    _unused: u32,
    /// TFTP IP address if applicable.
    pub tftp_ip: u32,
    /// TFTP port if applicable.
    pub tftp_port: u32,
    /// Partition index if applicable.
    pub partition_index: u32,
    /// MBR disk ID if applicable.
    pub mbr_disk_id: u32,
    /// GPT disk UUID if applicable.
    pub gpt_disk_uuid: Uuid,
    /// GPT partition UUID if applicable.
    pub gpt_part_uuid: Uuid,
    /// Partition UUID if applicable.
    pub part_uuid: Uuid,
}

impl File {
    /// Returns the path of the file as a Rust string slice.
    #[must_use]
    pub fn path(&self) -> &str {
        if self.path.is_null() {
            return "";
        }
        // SAFETY: The bootloader provides a valid null-terminated C string for file paths.
        // We checked for null above.
        unsafe {
            let c_str = core::ffi::CStr::from_ptr(self.path);
            c_str.to_str().unwrap_or("")
        }
    }

    /// Returns the name (string) associated with this file as a Rust string slice.
    #[must_use]
    pub fn name(&self) -> &str {
        if self.string.is_null() {
            return "";
        }
        // SAFETY: The bootloader provides a valid null-terminated C string for file names.
        // We checked for null above.
        unsafe {
            let c_str = core::ffi::CStr::from_ptr(self.string);
            c_str.to_str().unwrap_or("")
        }
    }
}

/// Iterator over a list of files.
///
/// Used internally to iterate over files provided by the bootloader.
pub struct FileIter<'a> {
    files: &'a [NonNull<File>],
    index: usize,
}

impl FileIter<'_> {
    /// Creates a new `FileIter`.
    pub(crate) fn new(files: NonNull<NonNull<File>>, count: usize) -> FileIter<'static> {
        // SAFETY: The bootloader provides a valid pointer to an array of `count`
        // NonNull<File> pointers.
        let files_slice = unsafe { core::slice::from_raw_parts(files.as_ptr(), count) };
        FileIter {
            files: files_slice,
            index: 0,
        }
    }
}

impl<'a> Iterator for FileIter<'a> {
    type Item = &'a File;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.files.len() {
            return None;
        }
        // SAFETY: Each NonNull<File> in the slice was provided by the bootloader and points
        // to a valid File structure that lives for the lifetime of the kernel.
        let file = unsafe { self.files[self.index].as_ref() };
        self.index += 1;
        Some(file)
    }
}
