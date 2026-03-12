//! VFS wire protocol shared between kernel and userspace.
//!
//! Defines `#[repr(C)]` message types for the filesystem server protocol.
//! All types are layout-stable for crossing IPC boundaries via channels.

#![no_std]

// ── Op codes ────────────────────────────────────────────────────────

/// Open a file or directory.
pub const FS_OP_OPEN: u32 = 0x01;
/// Read from an open file.
pub const FS_OP_READ: u32 = 0x02;
/// Write to an open file.
pub const FS_OP_WRITE: u32 = 0x03;
/// Get file metadata.
pub const FS_OP_STAT: u32 = 0x04;
/// Read directory entries.
pub const FS_OP_READDIR: u32 = 0x05;
/// Close (reserved — close is implicit via `PEER_CLOSED`).
pub const FS_OP_CLOSE: u32 = 0x06;
/// Delete a directory entry.
pub const FS_OP_UNLINK: u32 = 0x07;
/// Seek within an open file.
pub const FS_OP_SEEK: u32 = 0x08;
/// Create a directory.
pub const FS_OP_MKDIR: u32 = 0x09;
/// Rename a file or directory.
pub const FS_OP_RENAME: u32 = 0x0A;
/// Create a symbolic link.
pub const FS_OP_SYMLINK: u32 = 0x0B;
/// Create a hard link.
pub const FS_OP_LINK: u32 = 0x0C;
/// Read a symbolic link target.
pub const FS_OP_READLINK: u32 = 0x0D;
/// Truncate a file.
pub const FS_OP_TRUNCATE: u32 = 0x0E;
/// Stat relative to a directory fd.
pub const FS_OP_FSTATAT: u32 = 0x0F;

// ── Request / reply headers ─────────────────────────────────────────

/// VFS request header (12 bytes).
///
/// Sent as the first bytes of every VFS request message. Followed by
/// `path_len` bytes of path data, then op-specific argument structs.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VfsRequest {
    /// Operation code (`FS_OP_*`).
    pub op: u32,
    /// Operation-specific flags (e.g., open flags).
    pub flags: u32,
    /// Length of the path string that follows this header.
    pub path_len: u32,
}

/// VFS reply header (8 bytes).
///
/// Sent as the first bytes of every VFS reply message. `status == 0`
/// indicates success; a positive value is an errno code.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VfsReply {
    /// 0 = success, positive = errno error code.
    pub status: i32,
    /// Number of bytes of payload following this header.
    pub data_len: u32,
}

// ── Per-op argument structs ─────────────────────────────────────────

/// Arguments for `FS_OP_READ`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ReadArgs {
    /// Byte offset to read from.
    pub offset: u64,
    /// Maximum number of bytes to read.
    pub len: u64,
}

/// Arguments for `FS_OP_WRITE`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct WriteArgs {
    /// Byte offset to write at.
    pub offset: u64,
}

/// Arguments for `FS_OP_READDIR`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ReaddirArgs {
    /// Entry index offset (0 = first entry).
    pub offset: u64,
    /// Maximum number of entries to return.
    pub max_entries: u32,
    /// Padding for alignment.
    pub _pad: u32,
}

/// Reply for large reads backed by a VMO transfer.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VmoReadReply {
    /// Actual number of bytes available in the VMO.
    pub actual: u64,
}

// ── Serialization helpers ───────────────────────────────────────────

/// Convert a `#[repr(C)]` value to its byte representation.
///
/// # Safety
///
/// `T` must be `#[repr(C)]` with no padding bytes containing uninitialized
/// data. All types in this crate satisfy this requirement.
pub unsafe fn as_bytes<T: Copy>(val: &T) -> &[u8] {
    // SAFETY: Caller guarantees T is repr(C) with no uninit padding.
    unsafe {
        core::slice::from_raw_parts(
            core::ptr::from_ref(val).cast::<u8>(),
            core::mem::size_of::<T>(),
        )
    }
}

/// Interpret a byte slice as a `#[repr(C)]` value.
///
/// Returns `None` if the slice is too short or misaligned.
pub fn from_bytes<T: Copy>(bytes: &[u8]) -> Option<&T> {
    if bytes.len() < core::mem::size_of::<T>() {
        return None;
    }
    let ptr = bytes.as_ptr();
    if ptr.align_offset(core::mem::align_of::<T>()) != 0 {
        return None;
    }
    // SAFETY: We checked length and alignment. T is Copy + repr(C).
    Some(unsafe { &*ptr.cast::<T>() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem;

    #[test]
    fn vfs_request_layout() {
        assert_eq!(mem::size_of::<VfsRequest>(), 12);
        assert_eq!(mem::align_of::<VfsRequest>(), 4);
    }

    #[test]
    fn vfs_reply_layout() {
        assert_eq!(mem::size_of::<VfsReply>(), 8);
        assert_eq!(mem::align_of::<VfsReply>(), 4);
    }

    #[test]
    fn read_args_layout() {
        assert_eq!(mem::size_of::<ReadArgs>(), 16);
        assert_eq!(mem::align_of::<ReadArgs>(), 8);
    }

    #[test]
    fn write_args_layout() {
        assert_eq!(mem::size_of::<WriteArgs>(), 8);
        assert_eq!(mem::align_of::<WriteArgs>(), 8);
    }

    #[test]
    fn readdir_args_layout() {
        assert_eq!(mem::size_of::<ReaddirArgs>(), 16);
        assert_eq!(mem::align_of::<ReaddirArgs>(), 8);
    }

    #[test]
    fn vmo_read_reply_layout() {
        assert_eq!(mem::size_of::<VmoReadReply>(), 8);
        assert_eq!(mem::align_of::<VmoReadReply>(), 8);
    }

    #[test]
    fn op_codes_unique() {
        let ops: &[u32] = &[
            FS_OP_OPEN,
            FS_OP_READ,
            FS_OP_WRITE,
            FS_OP_STAT,
            FS_OP_READDIR,
            FS_OP_CLOSE,
            FS_OP_UNLINK,
            FS_OP_SEEK,
            FS_OP_MKDIR,
            FS_OP_RENAME,
            FS_OP_SYMLINK,
            FS_OP_LINK,
            FS_OP_READLINK,
            FS_OP_TRUNCATE,
            FS_OP_FSTATAT,
        ];
        for (i, a) in ops.iter().enumerate() {
            for (j, b) in ops.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "duplicate op code {a:#x} at indices {i} and {j}");
                }
            }
        }
    }

    #[test]
    fn round_trip_vfs_request() {
        let req = VfsRequest {
            op: FS_OP_OPEN,
            flags: 0x42,
            path_len: 5,
        };
        // SAFETY: VfsRequest is repr(C) with no padding.
        let bytes = unsafe { as_bytes(&req) };
        let decoded: &VfsRequest = from_bytes(bytes).expect("decode failed");
        assert_eq!(decoded.op, FS_OP_OPEN);
        assert_eq!(decoded.flags, 0x42);
        assert_eq!(decoded.path_len, 5);
    }

    #[test]
    fn round_trip_vfs_reply() {
        let reply = VfsReply {
            status: 0,
            data_len: 128,
        };
        // SAFETY: VfsReply is repr(C) with no padding.
        let bytes = unsafe { as_bytes(&reply) };
        let decoded: &VfsReply = from_bytes(bytes).expect("decode failed");
        assert_eq!(decoded.status, 0);
        assert_eq!(decoded.data_len, 128);
    }

    #[test]
    fn from_bytes_too_short() {
        let bytes = [0u8; 4];
        assert!(from_bytes::<VfsRequest>(&bytes).is_none());
    }
}
