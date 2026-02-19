//! VFS mount table and path resolution.
//!
//! The VFS maintains a table of mounted filesystems keyed by mount path.
//! Path resolution finds the longest-matching mount point, then walks
//! remaining path components via [`Inode::lookup`].

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;

use hadron_core::sync::SpinLock;

use super::path;
use super::{FileSystem, FsError, Inode, poll_immediate};

/// Global VFS instance.
static VFS: SpinLock<Option<Vfs>> = SpinLock::new(None);

/// The virtual filesystem mount table.
pub struct Vfs {
    /// Mount points mapping path -> filesystem.
    mounts: BTreeMap<String, Arc<dyn FileSystem>>,
}

impl Vfs {
    /// Creates a new empty VFS.
    fn new() -> Self {
        Self {
            mounts: BTreeMap::new(),
        }
    }

    /// Mount a filesystem at the given path.
    pub fn mount(&mut self, path: &str, fs: Arc<dyn FileSystem>) {
        hadron_core::kinfo!("VFS: Mounted {} at {}", fs.name(), path);
        self.mounts.insert(path.to_string(), fs);
    }

    /// Resolve an absolute path to an inode.
    ///
    /// Finds the longest mount prefix, obtains the root inode, then walks
    /// the remaining path components via `lookup`.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::InvalidArgument`] if the path is not absolute,
    /// or [`FsError::NotFound`] if the path cannot be resolved.
    pub fn resolve(&self, abs_path: &str) -> Result<Arc<dyn Inode>, FsError> {
        if !path::is_absolute(abs_path) {
            return Err(FsError::InvalidArgument);
        }

        let mount_path =
            path::longest_prefix_match(abs_path, self.mounts.keys().map(String::as_str))
                .ok_or(FsError::NotFound)?;

        let fs = self.mounts.get(mount_path).ok_or(FsError::NotFound)?;
        let root = fs.root();

        let remainder = path::strip_mount_prefix(abs_path, mount_path);
        if remainder.is_empty() {
            return Ok(root);
        }

        let mut current = root;
        for component in path::components(remainder) {
            current = poll_immediate(current.lookup(component))?;
        }

        Ok(current)
    }
}

/// Initialize the global VFS.
///
/// # Panics
///
/// Panics if the VFS has already been initialized.
pub fn init() {
    let mut vfs = VFS.lock();
    assert!(vfs.is_none(), "VFS already initialized");
    *vfs = Some(Vfs::new());
}

/// Execute a closure with a shared reference to the global VFS.
///
/// # Panics
///
/// Panics if the VFS has not been initialized.
pub fn with_vfs<R>(f: impl FnOnce(&Vfs) -> R) -> R {
    let vfs = VFS.lock();
    f(vfs.as_ref().expect("VFS not initialized"))
}

/// Execute a closure with a mutable reference to the global VFS.
///
/// # Panics
///
/// Panics if the VFS has not been initialized.
pub fn with_vfs_mut<R>(f: impl FnOnce(&mut Vfs) -> R) -> R {
    let mut vfs = VFS.lock();
    f(vfs.as_mut().expect("VFS not initialized"))
}
