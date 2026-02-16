//! Memory Map Entry definitions and iterator.
//!
//! This module provides types for working with the system memory map provided by
//! the Limine bootloader. The memory map describes all physical memory regions
//! and their types (usable, reserved, etc.).
//!
//! # Overview
//!
//! The memory map is essential for:
//! - Initializing the kernel's memory allocator
//! - Understanding which memory regions are available for use
//! - Avoiding memory regions used by hardware or firmware
//!
//! # Memory Entry Types
//!
//! Different memory regions have different purposes:
//! - [`MemMapEntryType::Usable`] - Normal RAM available for use
//! - [`MemMapEntryType::Reserved`] - Reserved by firmware or hardware
//! - [`MemMapEntryType::AcpiReclaimable`] - ACPI tables (can be reclaimed after parsing)
//! - [`MemMapEntryType::AcpiNvs`] - ACPI NVS memory (must not be used)
//! - [`MemMapEntryType::BadMemory`] - Defective memory regions
//! - [`MemMapEntryType::BootloaderReclaimable`] - Used by bootloader (can be reclaimed)
//! - [`MemMapEntryType::KernelAndModules`] - Contains kernel and module data
//! - [`MemMapEntryType::Framebuffer`] - Framebuffer memory
//!
//! # Example
//!
//! ```no_run
//! use limine::{MemMapRequest, memmap::MemMapEntryType};
//!
//! #[used]
//! #[link_section = ".requests"]
//! static MEMMAP_REQUEST: MemMapRequest = MemMapRequest::new();
//!
//! fn find_usable_memory() -> u64 {
//!     let mut total_usable = 0u64;
//!     if let Some(memmap_response) = MEMMAP_REQUEST.response() {
//!         for entry in memmap_response.entries() {
//!             if entry.type_ == MemMapEntryType::Usable {
//!                 total_usable += entry.length;
//!             }
//!         }
//!     }
//!     total_usable
//! }
//! ```

use core::ptr::NonNull;

/// The type of a memory map entry.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemMapEntryType {
    /// Usable RAM.
    Usable = 0,
    /// Reserved memory.
    Reserved = 1,
    /// ACPI Reclaimable memory.
    AcpiReclaimable = 2,
    /// ACPI NVS memory.
    AcpiNvs = 3,
    /// Bad memory.
    BadMemory = 4,
    /// Bootloader Reclaimable memory.
    BootloaderReclaimable = 5,
    /// Kernel and modules memory.
    KernelAndModules = 6,
    /// Framebuffer memory.
    Framebuffer = 7,
    /// ACPI Tables memory.
    AcpiTables = 8,
}

/// A single entry in the memory map.
#[repr(C)]
pub struct MemMapEntry {
    /// The starting physical address of the memory region.
    pub base: u64,
    /// The length of the memory region in bytes.
    pub length: u64,
    /// The type of the memory region.
    pub type_: MemMapEntryType,
}

/// An iterator over memory map entries.
pub struct MemMapIter<'a> {
    entries: &'a [NonNull<MemMapEntry>],
    index: usize,
}

impl MemMapIter<'_> {
    /// Creates a new memory map iterator.
    pub(crate) fn new(
        entry_count: usize,
        entries: NonNull<NonNull<MemMapEntry>>,
    ) -> MemMapIter<'static> {
        // SAFETY: The bootloader provides a valid pointer to an array of `entry_count`
        // NonNull<MemMapEntry> pointers.
        let entries_slice = unsafe { core::slice::from_raw_parts(entries.as_ptr(), entry_count) };
        MemMapIter {
            entries: entries_slice,
            index: 0,
        }
    }
}

impl Iterator for MemMapIter<'_> {
    type Item = &'static MemMapEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.entries.len() {
            return None;
        }
        let entry_ptr = self.entries[self.index];
        self.index += 1;
        // SAFETY: Each NonNull<MemMapEntry> in the slice was provided by the bootloader
        // and points to a valid MemMapEntry structure that lives for the lifetime of the kernel.
        Some(unsafe { entry_ptr.as_ref() })
    }
}
