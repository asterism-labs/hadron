//! Device filesystem (`/dev`).
//!
//! Provides virtual device files:
//! - `/dev/null` -- reads return 0 bytes, writes are discarded
//! - `/dev/zero` -- reads fill buffer with zeros, writes are discarded
//!
//! Additional devices are registered at runtime via [`DevFsDir::insert`] or the
//! kernel-side `devfs_registry` module.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use hadron_core::sync::SpinLock;

use crate::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

// ── DevNumber ───────────────────────────────────────────────────────────

/// Linux-compatible device number (major:minor) encoded as a single `u64`.
///
/// Encoding follows Linux `makedev(major, minor)`:
/// - bits 7..0    — minor bits 7..0
/// - bits 19..8   — major bits 11..0
/// - bits 31..20  — minor bits 19..8
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevNumber(pub u64);

impl DevNumber {
    /// Create a device number from `major` and `minor`.
    #[must_use]
    pub const fn new(major: u32, minor: u32) -> Self {
        let lo = (minor & 0xff) as u64;
        let hi = ((minor >> 8) as u64) << 20;
        let maj = ((major & 0xfff) as u64) << 8;
        Self(lo | maj | hi)
    }

    /// Extract the major number.
    #[must_use]
    pub const fn major(self) -> u32 {
        ((self.0 >> 8) & 0xfff) as u32
    }

    /// Extract the minor number.
    #[must_use]
    pub const fn minor(self) -> u32 {
        let lo = (self.0 & 0xff) as u32;
        let hi = ((self.0 >> 20) & 0xfff) as u32;
        lo | (hi << 8)
    }

    // ── Named constants ──

    /// `/dev/null` — major 1, minor 3.
    pub const NULL: Self = Self::new(1, 3);
    /// `/dev/zero` — major 1, minor 5.
    pub const ZERO: Self = Self::new(1, 5);
    /// `/dev/console` — major 5, minor 1.
    pub const CONSOLE: Self = Self::new(5, 1);
    /// `/dev/ptmx` — major 5, minor 2.
    pub const PTMX: Self = Self::new(5, 2);

    /// `/dev/ttyN` — major 4, minor N.
    #[must_use]
    pub const fn tty_vt(n: u32) -> Self {
        Self::new(4, n)
    }

    /// `/dev/pts/N` — major 136, minor N.
    #[must_use]
    pub const fn pts(n: u32) -> Self {
        Self::new(136, n)
    }

    /// `/dev/fbN` — major 29, minor N.
    #[must_use]
    pub const fn fb(n: u32) -> Self {
        Self::new(29, n)
    }

    /// `/dev/dri/cardN` — major 226, minor N.
    #[must_use]
    pub const fn drm_card(n: u32) -> Self {
        Self::new(226, n)
    }

    /// `/dev/dri/renderDN` — major 226, minor 128 + N.
    #[must_use]
    pub const fn drm_render(n: u32) -> Self {
        Self::new(226, 128 + n)
    }

    /// `/dev/sdX` (SCSI block device) — major 8, minor N.
    #[must_use]
    pub const fn block_scsi(n: u32) -> Self {
        Self::new(8, n)
    }
}

// ── DevFs ───────────────────────────────────────────────────────────────

/// The devfs filesystem.
pub struct DevFs {
    /// Root directory.
    root: Arc<DevFsDir>,
}

impl Default for DevFs {
    fn default() -> Self {
        Self::new()
    }
}

impl DevFs {
    /// Creates a new devfs with standard device entries (`null`, `zero`).
    #[must_use]
    pub fn new() -> Self {
        let root = DevFsDir::new();
        root.insert("null".into(), Arc::new(DevNull));
        root.insert("zero".into(), Arc::new(DevZero));
        Self { root }
    }

    /// Returns a clone of the root [`DevFsDir`] for dynamic device registration.
    pub fn root_dir(&self) -> Arc<DevFsDir> {
        self.root.clone()
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

// ── DevFsDirInner ───────────────────────────────────────────────────────

/// Interior state of a devfs directory.
struct DevFsDirInner {
    /// Non-directory device entries.
    devices: BTreeMap<String, Arc<dyn Inode>>,
    /// Subdirectory entries (kept separately for safe `get_or_create_dir`).
    subdirs: BTreeMap<String, Arc<DevFsDir>>,
}

impl DevFsDirInner {
    const fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            subdirs: BTreeMap::new(),
        }
    }
}

// ── DevFsDir ────────────────────────────────────────────────────────────

/// A devfs directory with runtime-mutable entries.
///
/// Uses a [`SpinLock`] for interior mutability so devices can be registered
/// after the filesystem is mounted.
pub struct DevFsDir {
    inner: SpinLock<DevFsDirInner>,
}

impl DevFsDir {
    /// Creates an empty devfs directory.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: SpinLock::leveled("devfs_dir", 5, DevFsDirInner::new()),
        })
    }

    /// Insert (or replace) a device entry. Returns the previous inode, if any.
    ///
    /// If `inode` is itself a [`DevFsDir`], use [`get_or_create_dir`] instead
    /// for proper subdirectory tracking.
    pub fn insert(&self, name: String, inode: Arc<dyn Inode>) -> Option<Arc<dyn Inode>> {
        self.inner.lock().devices.insert(name, inode)
    }

    /// Remove a device entry. Returns the removed inode, if any.
    pub fn remove(&self, name: &str) -> Option<Arc<dyn Inode>> {
        let mut inner = self.inner.lock();
        // Check both maps.
        if let removed @ Some(_) = inner.devices.remove(name) {
            return removed;
        }
        inner.subdirs.remove(name).map(|arc| arc as Arc<dyn Inode>)
    }

    /// Return the existing subdirectory named `name`, or create a new empty
    /// [`DevFsDir`] and insert it, then return it.
    pub fn get_or_create_dir(&self, name: &str) -> Arc<DevFsDir> {
        let mut inner = self.inner.lock();
        if let Some(existing) = inner.subdirs.get(name) {
            return existing.clone();
        }
        let dir = DevFsDir::new();
        inner.subdirs.insert(name.to_string(), dir.clone());
        dir
    }
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

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let inner = self.inner.lock();
            // Check subdirs first, then devices.
            if let Some(dir) = inner.subdirs.get(name) {
                return Ok(dir.clone() as Arc<dyn Inode>);
            }
            inner.devices.get(name).cloned().ok_or(FsError::NotFound)
        })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async move {
            let inner = self.inner.lock();
            let mut entries = Vec::with_capacity(inner.devices.len() + inner.subdirs.len());
            for name in inner.subdirs.keys() {
                entries.push(DirEntry {
                    name: name.clone(),
                    inode_type: InodeType::Directory,
                });
            }
            for (name, inode) in &inner.devices {
                entries.push(DirEntry {
                    name: name.clone(),
                    inode_type: inode.inode_type(),
                });
            }
            Ok(entries)
        })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn unlink<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async move {
            let mut inner = self.inner.lock();
            if inner.devices.remove(name).is_some() {
                return Ok(());
            }
            if inner.subdirs.remove(name).is_some() {
                return Ok(());
            }
            Err(FsError::NotFound)
        })
    }

    fn as_devfs_dir(&self) -> Option<&DevFsDir> {
        Some(self)
    }
}

// ── /dev/null ──────────────────────────────────────────────────────────

/// `/dev/null` -- reads return EOF, writes are silently discarded.
pub struct DevNull;

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

    fn dev_number(&self) -> crate::DevNumber {
        crate::DevNumber::NULL
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

    fn lookup<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }
}

// ── /dev/zero ──────────────────────────────────────────────────────────

/// `/dev/zero` -- reads fill the buffer with zeros, writes are discarded.
pub struct DevZero;

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

    fn dev_number(&self) -> crate::DevNumber {
        crate::DevNumber::ZERO
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

    fn lookup<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }
}
