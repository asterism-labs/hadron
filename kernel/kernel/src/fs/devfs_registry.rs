//! Global devfs device registration API.
//!
//! Drivers call [`register_device`] after probe to publish their inodes under
//! `/dev`. The registry splits the path on `/`, walks or creates intermediate
//! [`DevFsDir`] nodes via [`DevFsDir::get_or_create_dir`], and inserts the
//! leaf inode.
//!
//! [`set_root`] must be called once during early boot before any driver can
//! register devices.

extern crate alloc;

use alloc::sync::Arc;

use hadron_core::sync::SpinLock;
use hadron_fs::Inode;
use hadron_fs::devfs::DevFsDir;

/// The global devfs root directory.
static DEVFS_ROOT: SpinLock<Option<Arc<DevFsDir>>> = SpinLock::leveled("devfs_root", 5, None);

/// Set the global devfs root directory.
///
/// Must be called exactly once during boot, before any calls to
/// [`register_device`] or [`unregister_device`].
///
/// # Panics
///
/// Panics if the root has already been set.
pub fn set_root(root: Arc<DevFsDir>) {
    let mut guard = DEVFS_ROOT.lock();
    assert!(guard.is_none(), "devfs_registry: root already set");
    *guard = Some(root);
}

/// Register a device inode at `path` under `/dev`.
///
/// `path` must be a relative path without a leading `/`, e.g.
/// `"dri/renderD128"` for `/dev/dri/renderD128`. Intermediate directories
/// are created automatically.
///
/// # Panics
///
/// Panics if the devfs root has not been set yet.
pub fn register_device(path: &str, inode: Arc<dyn Inode>) {
    let root = {
        let guard = DEVFS_ROOT.lock();
        guard
            .as_ref()
            .expect("devfs_registry: root not set")
            .clone()
    };

    // Split path into components, e.g. "dri/renderD128" → ["dri", "renderD128"].
    let mut components = path.split('/').peekable();
    let mut current = root;

    loop {
        let component = components
            .next()
            .expect("devfs_registry: path must not be empty");
        if components.peek().is_none() {
            // Leaf: insert the inode.
            current.insert(component.into(), inode);
            return;
        }
        // Intermediate directory: walk or create.
        let next = current.get_or_create_dir(component);
        current = next;
    }
}

/// Unregister the device at `path` under `/dev`.
///
/// Returns `true` if an entry was removed, `false` if the path was not found.
///
/// # Panics
///
/// Panics if the devfs root has not been set yet.
pub fn unregister_device(path: &str) -> bool {
    let root = {
        let guard = DEVFS_ROOT.lock();
        guard
            .as_ref()
            .expect("devfs_registry: root not set")
            .clone()
    };

    // Walk to the parent directory.
    let (parent_path, leaf) = match path.rfind('/') {
        Some(idx) => (&path[..idx], &path[idx + 1..]),
        None => ("", path),
    };

    if parent_path.is_empty() {
        // Top-level entry.
        return root.remove(leaf).is_some();
    }

    // Walk the parent path.
    let mut current = root;
    for component in parent_path.split('/') {
        let inner = current.get_or_create_dir(component);
        current = inner;
    }
    current.remove(leaf).is_some()
}
