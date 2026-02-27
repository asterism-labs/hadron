//! Sysfs virtual filesystem (`/sys`).
//!
//! Provides a minimal `/sys` tree that satisfies Mesa's DRM loader scan:
//! - `/sys/bus/pci/devices/<addr>/` — per-PCI-device attribute directories
//! - `/sys/class/drm/` — DRM device symlinks (populated by GPU drivers)
//!
//! All nodes are read-only; writes and creates return `EACCES`/`ENOSYS`.
//! File content is generated at population time and stored as a `Vec<u8>`.
//!
//! # Population
//!
//! Call [`crate::fs::sysfs_registry::populate_pci`] after PCI enumeration.
//! GPU drivers call [`crate::fs::sysfs_registry::register_drm`] to add DRM
//! symlinks under `/sys/class/drm/`.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use hadron_core::sync::SpinLock;

use crate::fs::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

// ── SysFs ────────────────────────────────────────────────────────────────────

/// The sysfs filesystem.
pub struct SysFs {
    root: Arc<SysDir>,
}

impl SysFs {
    /// Create a new sysfs instance with the standard directory skeleton.
    ///
    /// Pre-creates: `/bus/`, `/bus/pci/`, `/bus/pci/devices/`,
    /// `/class/`, `/class/drm/`.
    #[must_use]
    pub fn new() -> Self {
        let root = SysDir::new();

        // /sys/bus/pci/devices/
        let bus = root.get_or_create_dir("bus");
        let bus_pci = bus.get_or_create_dir("pci");
        bus_pci.get_or_create_dir("devices");

        // /sys/class/drm/
        let class = root.get_or_create_dir("class");
        class.get_or_create_dir("drm");

        Self { root }
    }

    /// Return the root [`SysDir`] for population.
    pub fn root_dir(&self) -> Arc<SysDir> {
        self.root.clone()
    }
}

impl Default for SysFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for SysFs {
    fn name(&self) -> &'static str {
        "sysfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

// ── SysDirInner ──────────────────────────────────────────────────────────────

/// Interior state of a sysfs directory.
struct SysDirInner {
    /// Non-directory file/symlink entries.
    files: BTreeMap<String, Arc<dyn Inode>>,
    /// Subdirectory entries (kept separately for `get_or_create_dir`).
    subdirs: BTreeMap<String, Arc<SysDir>>,
}

impl SysDirInner {
    const fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            subdirs: BTreeMap::new(),
        }
    }
}

// ── SysDir ───────────────────────────────────────────────────────────────────

/// A sysfs directory node with mutable entries.
///
/// Backed by two maps: `files` (attributes/symlinks) and `subdirs` (child
/// directories). This separation avoids any need for `Arc<dyn Inode>` to
/// `Arc<SysDir>` downcasting in [`get_or_create_dir`].
pub struct SysDir {
    inner: SpinLock<SysDirInner>,
}

impl SysDir {
    /// Create a new, empty sysfs directory.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: SpinLock::leveled("sysfs_dir", 4, SysDirInner::new()),
        })
    }

    /// Insert a named file or symlink inode.
    pub fn insert(&self, name: String, inode: Arc<dyn Inode>) -> Option<Arc<dyn Inode>> {
        self.inner.lock().files.insert(name, inode)
    }

    /// Walk to or lazily create an intermediate subdirectory named `name`.
    pub fn get_or_create_dir(&self, name: &str) -> Arc<SysDir> {
        let mut guard = self.inner.lock();
        if let Some(existing) = guard.subdirs.get(name) {
            return existing.clone();
        }
        let dir = SysDir::new();
        guard.subdirs.insert(name.to_string(), dir.clone());
        dir
    }
}

impl Inode for SysDir {
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
        Box::pin(async { Err(FsError::PermissionDenied) })
    }

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        let inner = self.inner.lock();
        // Check files first, then subdirs.
        let result: Result<Arc<dyn Inode>, FsError> = if let Some(inode) = inner.files.get(name) {
            Ok(inode.clone())
        } else if let Some(dir) = inner.subdirs.get(name) {
            Ok(dir.clone() as Arc<dyn Inode>)
        } else {
            Err(FsError::NotFound)
        };
        Box::pin(async move { result })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        let inner = self.inner.lock();
        let mut entries: Vec<DirEntry> = inner
            .files
            .iter()
            .map(|(name, inode)| DirEntry {
                name: name.clone(),
                inode_type: inode.inode_type(),
            })
            .collect();
        for name in inner.subdirs.keys() {
            entries.push(DirEntry {
                name: name.clone(),
                inode_type: InodeType::Directory,
            });
        }
        Box::pin(async move { Ok(entries) })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::PermissionDenied) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::PermissionDenied) })
    }
}

// ── SysAttrFile ──────────────────────────────────────────────────────────────

/// A read-only sysfs attribute file with static string content.
pub struct SysAttrFile {
    content: Vec<u8>,
}

impl SysAttrFile {
    /// Create a new attribute file.
    ///
    /// A trailing newline is appended if `content` does not end with one.
    pub fn new(content: impl Into<String>) -> Arc<Self> {
        let mut s: String = content.into();
        if !s.ends_with('\n') {
            s.push('\n');
        }
        Arc::new(Self {
            content: s.into_bytes(),
        })
    }
}

impl Inode for SysAttrFile {
    fn inode_type(&self) -> InodeType {
        InodeType::File
    }

    fn size(&self) -> usize {
        self.content.len()
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        let start = offset.min(self.content.len());
        let available = &self.content[start..];
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        Box::pin(async move { Ok(n) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::PermissionDenied) })
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

// ── SysSymlink ───────────────────────────────────────────────────────────────

/// A sysfs symlink node.
pub struct SysSymlink {
    target: String,
}

impl SysSymlink {
    /// Create a new symlink pointing to `target`.
    pub fn new(target: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            target: target.into(),
        })
    }
}

impl Inode for SysSymlink {
    fn inode_type(&self) -> InodeType {
        InodeType::Symlink
    }

    fn size(&self) -> usize {
        self.target.len()
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        let bytes = self.target.as_bytes();
        let start = offset.min(bytes.len());
        let available = &bytes[start..];
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        Box::pin(async move { Ok(n) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::PermissionDenied) })
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

    fn read_link(&self) -> Result<String, FsError> {
        Ok(self.target.clone())
    }
}
