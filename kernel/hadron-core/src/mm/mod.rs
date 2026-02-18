//! Memory management types and traits.

pub mod address_space;
pub mod heap;
pub mod hhdm;
pub mod layout;
pub mod mapper;
pub mod pmm;
pub mod region;
pub mod vmm;

use core::fmt;

use crate::addr::PhysAddr;
use crate::paging::{PageSize, PhysFrame};

/// Standard 4 KiB page size.
pub const PAGE_SIZE: usize = 4096;

/// Page offset mask (lower 12 bits).
pub const PAGE_MASK: usize = 0xFFF;

/// Zeroes a single page-sized region.
///
/// # Safety
///
/// `ptr` must point to a valid, writable, page-aligned region of at least
/// [`PAGE_SIZE`] bytes.
#[inline]
pub unsafe fn zero_frame(ptr: *mut u8) {
    unsafe { core::ptr::write_bytes(ptr, 0, PAGE_SIZE) };
}

/// A physical frame allocator.
///
/// # Safety
///
/// Implementations must return unique, properly-aligned physical frames that
/// are not in use elsewhere.
pub unsafe trait FrameAllocator<S: PageSize> {
    /// Allocates a single physical frame, returning `None` if out of memory.
    fn allocate_frame(&mut self) -> Option<PhysFrame<S>>;
}

/// A physical frame deallocator.
///
/// # Safety
///
/// Implementations must only deallocate frames that were previously allocated
/// by the corresponding allocator and are no longer in use.
pub unsafe trait FrameDeallocator<S: PageSize> {
    /// Returns a physical frame to the allocator.
    unsafe fn deallocate_frame(&mut self, frame: PhysFrame<S>);
}

/// A physical memory region descriptor, independent of bootloader types.
#[derive(Debug, Clone, Copy)]
pub struct PhysMemoryRegion {
    /// Physical start address of the region.
    pub start: PhysAddr,
    /// Size in bytes.
    pub size: u64,
    /// Whether this region is usable RAM.
    pub usable: bool,
}

/// PMM errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmmError {
    /// No free frames available.
    OutOfMemory,
    /// The frame address is invalid or out of range.
    InvalidFrame,
    /// The PMM has already been initialized.
    AlreadyInitialized,
    /// No usable region large enough for the bitmap was found.
    NoBitmapRegion,
}

impl fmt::Display for PmmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PmmError::OutOfMemory => write!(f, "out of physical memory"),
            PmmError::InvalidFrame => write!(f, "invalid frame address"),
            PmmError::AlreadyInitialized => write!(f, "PMM already initialized"),
            PmmError::NoBitmapRegion => write!(f, "no region large enough for bitmap"),
        }
    }
}

/// VMM errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmmError {
    /// The virtual address region is exhausted.
    RegionExhausted,
    /// Out of physical memory (PMM returned None).
    OutOfMemory,
    /// The page is not mapped.
    NotMapped,
    /// The page is already mapped.
    AlreadyMapped,
    /// Page size mismatch (e.g. tried to unmap a huge page as 4 KiB or vice versa).
    SizeMismatch,
}

impl fmt::Display for VmmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VmmError::RegionExhausted => write!(f, "virtual region exhausted"),
            VmmError::OutOfMemory => write!(f, "out of physical memory"),
            VmmError::NotMapped => write!(f, "page not mapped"),
            VmmError::AlreadyMapped => write!(f, "page already mapped"),
            VmmError::SizeMismatch => write!(f, "page size mismatch"),
        }
    }
}
