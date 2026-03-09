//! UEFI Block I/O Protocol.
//!
//! The Block I/O Protocol provides access to block devices (disks). It allows
//! reading and writing fixed-size blocks of data.

use crate::EfiStatus;

/// The Block I/O Protocol.
#[repr(C)]
pub struct BlockIoProtocol {
    /// The revision of this protocol. UEFI 2.1+ uses `EFI_BLOCK_IO_PROTOCOL_REVISION2`,
    /// and UEFI 2.2+ uses `EFI_BLOCK_IO_PROTOCOL_REVISION3`.
    pub revision: u64,
    /// Pointer to the media information for this device.
    pub media: *mut BlockIoMedia,
    /// Resets the block device hardware.
    pub reset: unsafe extern "efiapi" fn(
        this: *mut BlockIoProtocol,
        extended_verification: bool,
    ) -> EfiStatus,
    /// Reads the specified number of blocks from the device.
    pub read_blocks: unsafe extern "efiapi" fn(
        this: *mut BlockIoProtocol,
        media_id: u32,
        lba: u64,
        buffer_size: usize,
        buffer: *mut u8,
    ) -> EfiStatus,
    /// Writes the specified number of blocks to the device.
    pub write_blocks: unsafe extern "efiapi" fn(
        this: *mut BlockIoProtocol,
        media_id: u32,
        lba: u64,
        buffer_size: usize,
        buffer: *const u8,
    ) -> EfiStatus,
    /// Flushes all modified data to the physical block device.
    pub flush_blocks: unsafe extern "efiapi" fn(this: *mut BlockIoProtocol) -> EfiStatus,
}

/// Describes the characteristics of a block I/O device's media.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BlockIoMedia {
    /// The current media ID.
    pub media_id: u32,
    /// `true` if the media is removable.
    pub removable_media: bool,
    /// `true` if there is a media currently present in the device.
    pub media_present: bool,
    /// `true` if the block I/O was produced to abstract partition structures.
    pub logical_partition: bool,
    /// `true` if the media is read-only.
    pub read_only: bool,
    /// `true` if the `write_blocks` function caches the write data.
    pub write_caching: bool,
    /// The intrinsic block size of the device in bytes.
    pub block_size: u32,
    /// Supplies the alignment requirement for any buffer used in a data transfer.
    pub io_align: u32,
    /// The last LBA on the device (i.e., the number of logical blocks minus one).
    pub last_block: u64,

    // ── UEFI 2.2+ fields (revision 2+) ──────────────────────────
    /// The first LBA that is aligned on a physical block boundary.
    /// Only valid if `logical_partition` is `true`.
    pub lowest_aligned_lba: u64,
    /// The number of logical blocks per physical block.
    pub logical_blocks_per_physical_block: u32,

    // ── UEFI 2.3.1+ fields (revision 3+) ────────────────────────
    /// The optimal transfer length granularity in logical blocks.
    pub optimal_transfer_length_granularity: u32,
}

// ── Compile-time layout assertions ──────────────────────────────────

// BlockIoMedia has no pointers; sizes are architecture-independent.
const _: () = {
    assert!(core::mem::size_of::<BlockIoMedia>() == 48);
    assert!(core::mem::offset_of!(BlockIoMedia, media_id) == 0);
    assert!(core::mem::offset_of!(BlockIoMedia, removable_media) == 4);
    assert!(core::mem::offset_of!(BlockIoMedia, media_present) == 5);
    assert!(core::mem::offset_of!(BlockIoMedia, logical_partition) == 6);
    assert!(core::mem::offset_of!(BlockIoMedia, read_only) == 7);
    assert!(core::mem::offset_of!(BlockIoMedia, write_caching) == 8);
    // 3 bytes padding before block_size
    assert!(core::mem::offset_of!(BlockIoMedia, block_size) == 12);
    assert!(core::mem::offset_of!(BlockIoMedia, io_align) == 16);
    // 4 bytes padding before last_block
    assert!(core::mem::offset_of!(BlockIoMedia, last_block) == 24);
    assert!(core::mem::offset_of!(BlockIoMedia, lowest_aligned_lba) == 32);
    assert!(core::mem::offset_of!(BlockIoMedia, logical_blocks_per_physical_block) == 40);
    assert!(core::mem::offset_of!(BlockIoMedia, optimal_transfer_length_granularity) == 44);
};

#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<BlockIoProtocol>() == 48);
