//! FDT header types and big-endian primitives.

use hadron_binparse::FromBytes;

/// FDT magic number: `0xd00dfeed` in big-endian.
pub const FDT_MAGIC: u32 = 0xd00d_feed;

/// Minimum last-compatible version we support.
pub const FDT_MIN_COMPAT_VERSION: u32 = 16;

/// Big-endian 32-bit integer.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Be32(u32);

// SAFETY: `repr(transparent)` over `u32`, all bit patterns are valid.
unsafe impl FromBytes for Be32 {}

impl Be32 {
    /// Converts to native-endian `u32`.
    #[must_use]
    pub fn get(self) -> u32 {
        u32::from_be(self.0)
    }
}

/// Big-endian 64-bit integer.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub struct Be64(u64);

// SAFETY: `repr(transparent)` over `u64`, all bit patterns are valid.
unsafe impl FromBytes for Be64 {}

impl Be64 {
    /// Converts to native-endian `u64`.
    #[must_use]
    pub fn get(self) -> u64 {
        u64::from_be(self.0)
    }
}

/// Raw FDT header as laid out in the DTB blob (all fields big-endian).
#[derive(Clone, Copy, Debug, FromBytes)]
#[repr(C)]
pub struct RawFdtHeader {
    /// Must be [`FDT_MAGIC`] (`0xd00dfeed`).
    pub magic: Be32,
    /// Total size of the DTB blob in bytes.
    pub totalsize: Be32,
    /// Offset of the structure block from the start of the header.
    pub off_dt_struct: Be32,
    /// Offset of the strings block from the start of the header.
    pub off_dt_strings: Be32,
    /// Offset of the memory reservation block from the start of the header.
    pub off_mem_rsvmap: Be32,
    /// DTB version.
    pub version: Be32,
    /// Last compatible version.
    pub last_comp_version: Be32,
    /// Physical ID of the boot CPU.
    pub boot_cpuid_phys: Be32,
    /// Length in bytes of the strings block.
    pub size_dt_strings: Be32,
    /// Length in bytes of the structure block.
    pub size_dt_struct: Be32,
}
