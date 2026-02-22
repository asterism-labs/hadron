//! UEFI memory types and descriptors.
//!
//! This module provides types for working with UEFI memory services, including
//! memory allocation types, memory region types, and memory descriptors returned
//! by `GetMemoryMap`.
//!
//! # Memory Map Stride
//!
//! When iterating over memory descriptors returned by `GetMemoryMap`, callers
//! **must** use the `descriptor_size` value returned by the function as the stride
//! between entries, not `size_of::<EfiMemoryDescriptor>()`. The firmware may return
//! descriptors larger than the struct definition.

use bitflags::bitflags;

/// Specifies the type of allocation to perform in `AllocatePages` and `AllocatePool`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EfiAllocateType {
    /// Allocate any available range of pages that satisfies the request.
    AllocateAnyPages = 0,
    /// Allocate any available range of pages whose uppermost address is less than
    /// or equal to the specified address.
    AllocateMaxAddress = 1,
    /// Allocate pages at the specified address.
    AllocateAddress = 2,
}

/// The type of a memory region in the UEFI memory map.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EfiMemoryType {
    /// Not usable.
    ReservedMemoryType = 0,
    /// The code portions of a loaded UEFI application.
    LoaderCode = 1,
    /// The data portions of a loaded UEFI application.
    LoaderData = 2,
    /// The code portions of a loaded UEFI Boot Services Driver.
    BootServicesCode = 3,
    /// The data portions of a loaded UEFI Boot Services Driver.
    BootServicesData = 4,
    /// The code portions of a loaded UEFI Runtime Services Driver.
    RuntimeServicesCode = 5,
    /// The data portions of a loaded UEFI Runtime Services Driver.
    RuntimeServicesData = 6,
    /// Free (unallocated) memory.
    ConventionalMemory = 7,
    /// Memory in which errors have been detected.
    UnusableMemory = 8,
    /// Memory that holds the ACPI tables.
    AcpiReclaimMemory = 9,
    /// Address space reserved for use by the firmware.
    AcpiMemoryNvs = 10,
    /// Used by system firmware to request a memory-mapped I/O region.
    MemoryMappedIO = 11,
    /// System memory-mapped I/O region used to translate memory cycles to I/O cycles.
    MemoryMappedIOPortSpace = 12,
    /// Address space reserved by the firmware for code that is part of the processor.
    PalCode = 13,
    /// A memory region that operates as conventional memory but also supports
    /// byte-addressable non-volatility.
    PersistentMemory = 14,
    /// A memory region that describes system memory that has not been accepted
    /// by a call to the underlying isolation architecture.
    UnacceptedMemoryType = 15,
}

/// A descriptor for a region of physical memory as returned by `GetMemoryMap`.
///
/// # Layout Note
///
/// The firmware may return descriptors larger than this struct. Always use the
/// `descriptor_size` value from `GetMemoryMap` as the stride between entries.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EfiMemoryDescriptor {
    /// The type of this memory region.
    pub memory_type: u32,
    /// The physical address of the first byte in the memory region.
    pub physical_start: u64,
    /// The virtual address of the first byte in the memory region.
    pub virtual_start: u64,
    /// The number of 4 KiB pages in the memory region.
    pub number_of_pages: u64,
    /// Attributes of the memory region that describe the bit mask of capabilities
    /// for that memory region, and not necessarily the current settings for that
    /// memory region.
    pub attribute: u64,
}

bitflags! {
    /// Memory attribute flags for memory descriptors.
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EfiMemoryAttributes: u64 {
        /// Memory cacheability attribute: Uncacheable.
        const UC = 0x0000_0000_0000_0001;
        /// Memory cacheability attribute: Write Combining.
        const WC = 0x0000_0000_0000_0002;
        /// Memory cacheability attribute: Write Through.
        const WT = 0x0000_0000_0000_0004;
        /// Memory cacheability attribute: Write Back.
        const WB = 0x0000_0000_0000_0008;
        /// Memory cacheability attribute: Uncacheable, exported.
        const UCE = 0x0000_0000_0000_0010;
        /// Physical memory protection attribute: Write Protected.
        const WP = 0x0000_0000_0000_1000;
        /// Physical memory protection attribute: Read Protected.
        const RP = 0x0000_0000_0000_2000;
        /// Physical memory protection attribute: Execute Protected.
        const XP = 0x0000_0000_0000_4000;
        /// Non-volatile memory.
        const NV = 0x0000_0000_0000_8000;
        /// More reliable memory.
        const MORE_RELIABLE = 0x0000_0000_0001_0000;
        /// Memory region supports read-only protection.
        const RO = 0x0000_0000_0002_0000;
        /// Specific-purpose memory (SPM).
        const SP = 0x0000_0000_0004_0000;
        /// If set, the memory region is capable of being protected with CPU cryptographic
        /// capabilities.
        const CPU_CRYPTO = 0x0000_0000_0008_0000;
        /// Runtime memory attribute. If set, the memory region needs to be given a virtual
        /// mapping by the OS when `SetVirtualAddressMap()` is called.
        const RUNTIME = 0x8000_0000_0000_0000;
    }
}

// ── Compile-time layout assertions ──────────────────────────────────

// EfiMemoryDescriptor has no pointers; sizes are architecture-independent.
const _: () = {
    assert!(core::mem::size_of::<EfiMemoryDescriptor>() == 40);
    // 4 bytes of padding between memory_type (u32) and physical_start (u64)
    assert!(core::mem::offset_of!(EfiMemoryDescriptor, memory_type) == 0);
    assert!(core::mem::offset_of!(EfiMemoryDescriptor, physical_start) == 8);
    assert!(core::mem::offset_of!(EfiMemoryDescriptor, virtual_start) == 16);
    assert!(core::mem::offset_of!(EfiMemoryDescriptor, number_of_pages) == 24);
    assert!(core::mem::offset_of!(EfiMemoryDescriptor, attribute) == 32);
};
