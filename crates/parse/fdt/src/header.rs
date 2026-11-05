//! FDT header types.

use zerocopy::big_endian::U32;
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

/// FDT magic number: `0xd00dfeed` in big-endian.
pub const FDT_MAGIC: u32 = 0xd00d_feed;

/// Minimum last-compatible version we support.
pub const FDT_MIN_COMPAT_VERSION: u32 = 16;

/// Raw FDT header as laid out in the DTB blob (all fields big-endian).
#[derive(Clone, Copy, Debug, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C)]
pub struct RawFdtHeader {
    /// Must be [`FDT_MAGIC`] (`0xd00dfeed`).
    pub magic: U32,
    /// Total size of the DTB blob in bytes.
    pub totalsize: U32,
    /// Offset of the structure block from the start of the header.
    pub off_dt_struct: U32,
    /// Offset of the strings block from the start of the header.
    pub off_dt_strings: U32,
    /// Offset of the memory reservation block from the start of the header.
    pub off_mem_rsvmap: U32,
    /// DTB version.
    pub version: U32,
    /// Last compatible version.
    pub last_comp_version: U32,
    /// Physical ID of the boot CPU.
    pub boot_cpuid_phys: U32,
    /// Length in bytes of the strings block.
    pub size_dt_strings: U32,
    /// Length in bytes of the structure block.
    pub size_dt_struct: U32,
}
