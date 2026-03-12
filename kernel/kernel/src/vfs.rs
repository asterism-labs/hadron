//! Global VFS mount table with longest-prefix path resolution.
//!
//! The [`VfsRouter`] maintains a mapping from path prefixes to FS server
//! channel endpoints. When a vnode syscall resolves a path, the router finds
//! the longest matching mount prefix and returns the server channel plus the
//! remaining relative path.
//!
//! Zero filesystem logic lives in the kernel — path resolution, permissions,
//! and directory traversal are the FS server's responsibility.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicPtr, Ordering};

use hadron_core::sync::SpinLock;
use hadron_objects::channel::Channel;
use hadron_objects::object::{KernelObject, Koid};

/// Errors from VFS router operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsError {
    /// No mount matches the given path.
    NotFound,
    /// The mount point is not in the process's namespace.
    AccessDenied,
    /// A mount already exists at this prefix.
    AlreadyMounted,
}

/// Global mount table mapping path prefixes to FS server channels.
pub struct VfsRouter {
    /// Mount entries: normalized prefix → server channel.
    mounts: SpinLock<BTreeMap<String, Arc<Channel>>>,
}

/// Global VFS router instance. Initialized once, never replaced.
/// Only the inner `mounts` lock needs synchronization.
static VFS_ROUTER: AtomicPtr<VfsRouter> = AtomicPtr::new(core::ptr::null_mut());

/// Initialize the global VFS router. Called once during `kernel_init()`.
///
/// # Panics
///
/// Panics if called more than once.
pub fn init() {
    let router = Box::new(VfsRouter {
        mounts: SpinLock::leveled("VFS_ROUTER.mounts", 6, BTreeMap::new()),
    });
    let ptr = Box::into_raw(router);
    let prev = VFS_ROUTER.compare_exchange(
        core::ptr::null_mut(),
        ptr,
        Ordering::Release,
        Ordering::Relaxed,
    );
    assert!(prev.is_ok(), "VFS router already initialized");
}

/// Execute a closure with a reference to the global VFS router.
///
/// # Panics
///
/// Panics if the VFS router has not been initialized.
pub fn with<R>(f: impl FnOnce(&VfsRouter) -> R) -> R {
    let ptr = VFS_ROUTER.load(Ordering::Acquire);
    assert!(!ptr.is_null(), "VFS router not initialized");
    // SAFETY: ptr was set by `init()` from a valid Box and is never freed.
    let router = unsafe { &*ptr };
    f(router)
}

impl VfsRouter {
    /// Register a mount at the given normalized path prefix.
    ///
    /// The prefix must not have a trailing slash (except for `"/"`).
    ///
    /// # Errors
    ///
    /// Returns [`VfsError::AlreadyMounted`] if the prefix is already mounted.
    pub fn mount(&self, prefix: &str, channel: Arc<Channel>) -> Result<(), VfsError> {
        let mut mounts = self.mounts.lock();
        if mounts.contains_key(prefix) {
            return Err(VfsError::AlreadyMounted);
        }
        mounts.insert(String::from(prefix), channel);
        Ok(())
    }

    /// Remove the mount at the given prefix.
    ///
    /// # Errors
    ///
    /// Returns [`VfsError::NotFound`] if no mount exists at the prefix.
    pub fn unmount(&self, prefix: &str) -> Result<(), VfsError> {
        let mut mounts = self.mounts.lock();
        if mounts.remove(prefix).is_some() {
            Ok(())
        } else {
            Err(VfsError::NotFound)
        }
    }

    /// Resolve a path to a mount channel and relative path using
    /// longest-prefix matching, filtered by the process namespace.
    ///
    /// The `namespace` maps mount prefixes to channel koids. Only mounts
    /// whose channel koid appears in the namespace are considered.
    ///
    /// Returns `(server_channel, relative_path_after_prefix)`.
    ///
    /// # Errors
    ///
    /// - [`VfsError::NotFound`] if no mount prefix matches the path.
    /// - [`VfsError::AccessDenied`] if the matching mount is not in the
    ///   process's namespace.
    pub fn resolve(
        &self,
        path: &str,
        namespace: &BTreeMap<String, Koid>,
    ) -> Result<(Arc<Channel>, String), VfsError> {
        let mounts = self.mounts.lock();

        let mut best_prefix: Option<&str> = None;
        let mut best_channel: Option<&Arc<Channel>> = None;

        for (prefix, channel) in mounts.iter() {
            if !path_matches_prefix(path, prefix) {
                continue;
            }
            // Track the longest matching prefix.
            if best_prefix.is_none() || prefix.len() > best_prefix.unwrap().len() {
                best_prefix = Some(prefix.as_str());
                best_channel = Some(channel);
            }
        }

        let prefix = best_prefix.ok_or(VfsError::NotFound)?;
        let channel = best_channel.unwrap();

        // Check namespace: the channel's koid must be present in the
        // process namespace. The namespace stores channel koids directly.
        let ch_koid = channel.koid();
        let in_namespace = namespace.values().any(|koid| *koid == ch_koid);
        if !in_namespace {
            return Err(VfsError::AccessDenied);
        }

        let relative = relative_path(path, prefix);
        let channel_clone = Arc::clone(channel);
        drop(mounts);

        Ok((channel_clone, relative))
    }
}

/// Check if `path` matches the mount `prefix`.
///
/// A path matches if:
/// - `path == prefix` (exact match), or
/// - `prefix == "/"` (root always matches), or
/// - `path` starts with `prefix` followed by `/`
fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if prefix == "/" {
        return true;
    }
    if path == prefix {
        return true;
    }
    path.starts_with(prefix) && path.as_bytes().get(prefix.len()) == Some(&b'/')
}

/// Extract the relative path after stripping the mount prefix.
///
/// For root mount (`/`), returns the full path.
/// For other mounts, strips the prefix and returns the rest (including
/// leading `/`). If nothing remains, returns `"/"`.
fn relative_path(path: &str, prefix: &str) -> String {
    if prefix == "/" {
        return String::from(path);
    }
    let rest = &path[prefix.len()..];
    if rest.is_empty() {
        String::from("/")
    } else {
        String::from(rest)
    }
}

/// Normalize a VFS path by removing double slashes, trailing slashes, and
/// resolving `.` components (but NOT `..` — that's the FS server's job).
pub fn normalize_path(path: &str) -> String {
    let mut parts: alloc::vec::Vec<&str> = alloc::vec::Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return String::from("/");
    }
    let mut result = String::new();
    for part in &parts {
        result.push('/');
        result.push_str(part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_matches_root() {
        assert!(path_matches_prefix("/", "/"));
        assert!(path_matches_prefix("/foo", "/"));
        assert!(path_matches_prefix("/foo/bar", "/"));
    }

    #[test]
    fn path_matches_exact() {
        assert!(path_matches_prefix("/dev", "/dev"));
        assert!(path_matches_prefix("/data", "/data"));
    }

    #[test]
    fn path_matches_subpath() {
        assert!(path_matches_prefix("/dev/null", "/dev"));
        assert!(path_matches_prefix("/data/files", "/data"));
    }

    #[test]
    fn path_no_false_prefix_match() {
        // /datafile should NOT match /data — it must be followed by /
        assert!(!path_matches_prefix("/datafile", "/data"));
        assert!(!path_matches_prefix("/dev2", "/dev"));
    }

    #[test]
    fn relative_path_root_mount() {
        assert_eq!(relative_path("/foo/bar", "/"), "/foo/bar");
        assert_eq!(relative_path("/", "/"), "/");
    }

    #[test]
    fn relative_path_subpath() {
        assert_eq!(relative_path("/dev/null", "/dev"), "/null");
        assert_eq!(relative_path("/data/files/a", "/data"), "/files/a");
    }

    #[test]
    fn relative_path_exact_match() {
        assert_eq!(relative_path("/dev", "/dev"), "/");
    }

    #[test]
    fn normalize_removes_double_slashes() {
        assert_eq!(normalize_path("//foo//bar"), "/foo/bar");
    }

    #[test]
    fn normalize_removes_trailing_slash() {
        assert_eq!(normalize_path("/foo/bar/"), "/foo/bar");
    }

    #[test]
    fn normalize_removes_dot() {
        assert_eq!(normalize_path("/foo/./bar"), "/foo/bar");
    }

    #[test]
    fn normalize_root_only() {
        assert_eq!(normalize_path("/"), "/");
        assert_eq!(normalize_path("///"), "/");
    }
}
