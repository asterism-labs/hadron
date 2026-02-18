//! Process filesystem (`/proc`).
//!
//! Provides dynamic virtual files that expose kernel state:
//! - `/proc/meminfo` -- physical memory statistics

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use super::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

/// The procfs filesystem.
pub struct ProcFs {
    /// Root directory.
    root: Arc<ProcFsDir>,
}

impl Default for ProcFs {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcFs {
    /// Creates a new procfs with standard entries.
    #[must_use]
    pub fn new() -> Self {
        let mut entries: BTreeMap<&str, Arc<dyn Inode>> = BTreeMap::new();
        entries.insert("meminfo", Arc::new(ProcMeminfo));

        Self {
            root: Arc::new(ProcFsDir { entries }),
        }
    }
}

impl FileSystem for ProcFs {
    fn name(&self) -> &'static str {
        "procfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

/// The procfs root directory.
struct ProcFsDir {
    /// Fixed entries.
    entries: BTreeMap<&'static str, Arc<dyn Inode>>,
}

impl Inode for ProcFsDir {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError> {
        self.entries.get(name).cloned().ok_or(FsError::NotFound)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Ok(self
            .entries
            .iter()
            .map(|(name, inode)| DirEntry {
                name: (*name).to_string(),
                inode_type: inode.inode_type(),
            })
            .collect())
    }

    fn create(
        &self,
        _name: &str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotSupported)
    }

    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotSupported)
    }
}

// ── /proc/meminfo ──────────────────────────────────────────────────────

/// `/proc/meminfo` -- dynamically generates physical memory statistics.
struct ProcMeminfo;

/// Page size in bytes (4 KiB).
const PAGE_SIZE: usize = 4096;

impl Inode for ProcMeminfo {
    fn inode_type(&self) -> InodeType {
        InodeType::File
    }

    fn size(&self) -> usize {
        // Dynamic content; size not known until read.
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let (total, free) =
                crate::mm::pmm::with_pmm(|pmm| (pmm.total_frames(), pmm.free_frames()));

            let total_kb = total * PAGE_SIZE / 1024;
            let free_kb = free * PAGE_SIZE / 1024;
            let used_kb = total_kb - free_kb;

            let content = format!(
                "MemTotal:    {total_kb} kB\nMemFree:     {free_kb} kB\nMemUsed:     {used_kb} kB\n"
            );

            let bytes = content.as_bytes();
            if offset >= bytes.len() {
                return Ok(0);
            }
            let available = &bytes[offset..];
            let to_copy = buf.len().min(available.len());
            buf[..to_copy].copy_from_slice(&available[..to_copy]);
            Ok(to_copy)
        })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::PermissionDenied) })
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(
        &self,
        _name: &str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotADirectory)
    }
}
