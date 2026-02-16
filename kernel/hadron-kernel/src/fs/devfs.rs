//! Device filesystem (`/dev`).
//!
//! Provides virtual device files:
//! - `/dev/null` -- reads return 0 bytes, writes are discarded
//! - `/dev/zero` -- reads fill buffer with zeros, writes are discarded
//! - `/dev/console` -- writes go to kernel console, reads return 0

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use super::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

/// The devfs filesystem.
pub struct DevFs {
    /// Root directory containing device entries.
    root: Arc<DevFsDir>,
}

impl Default for DevFs {
    fn default() -> Self {
        Self::new()
    }
}

impl DevFs {
    /// Creates a new devfs with standard device entries.
    #[must_use]
    pub fn new() -> Self {
        let mut entries: BTreeMap<&str, Arc<dyn Inode>> = BTreeMap::new();
        entries.insert("null", Arc::new(DevNull));
        entries.insert("zero", Arc::new(DevZero));
        entries.insert("console", Arc::new(DevConsole));

        Self {
            root: Arc::new(DevFsDir { entries }),
        }
    }
}

impl FileSystem for DevFs {
    fn name(&self) -> &'static str {
        "devfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

/// The devfs root directory.
struct DevFsDir {
    /// Fixed device entries.
    entries: BTreeMap<&'static str, Arc<dyn Inode>>,
}

impl Inode for DevFsDir {
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

// ── /dev/null ──────────────────────────────────────────────────────────

/// `/dev/null` -- reads return EOF, writes are silently discarded.
struct DevNull;

impl Inode for DevNull {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Ok(0) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move { Ok(buf.len()) })
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

// ── /dev/zero ──────────────────────────────────────────────────────────

/// `/dev/zero` -- reads fill the buffer with zeros, writes are discarded.
struct DevZero;

impl Inode for DevZero {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            buf.fill(0);
            Ok(buf.len())
        })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move { Ok(buf.len()) })
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

// ── /dev/console ───────────────────────────────────────────────────────

/// `/dev/console` -- writes go to kernel console output, reads return EOF.
pub struct DevConsole;

impl Inode for DevConsole {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        // No keyboard input support yet -- return EOF.
        Box::pin(async { Ok(0) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if let Ok(s) = core::str::from_utf8(buf) {
                hadron_core::kprint!("{}", s);
            } else {
                for &byte in buf {
                    hadron_core::kprint!("{}", byte as char);
                }
            }
            Ok(buf.len())
        })
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
