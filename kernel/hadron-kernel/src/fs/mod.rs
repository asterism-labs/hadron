//! Virtual filesystem layer.
//!
//! Provides the [`Inode`] and [`FileSystem`] traits that abstract over different
//! filesystem implementations (ramfs, devfs, ext2, etc.). All file I/O goes
//! through these traits via the VFS mount table.

pub mod console_input;
pub mod devfs;
pub mod file;
pub mod initramfs;
pub mod path;
pub mod ramfs;
pub mod vfs;

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

/// File type of an inode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InodeType {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Character device.
    CharDevice,
}

/// File permissions.
#[derive(Debug, Clone, Copy)]
pub struct Permissions {
    /// Readable.
    pub read: bool,
    /// Writable.
    pub write: bool,
    /// Executable.
    pub execute: bool,
}

impl Permissions {
    /// Read-write-execute permissions.
    #[must_use]
    pub const fn all() -> Self {
        Self {
            read: true,
            write: true,
            execute: true,
        }
    }

    /// Read-only permissions.
    #[must_use]
    pub const fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            execute: false,
        }
    }

    /// Read-write permissions.
    #[must_use]
    pub const fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            execute: false,
        }
    }
}

/// A directory entry returned by [`Inode::readdir`].
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// Entry name.
    pub name: String,
    /// Type of the entry.
    pub inode_type: InodeType,
}

/// Filesystem error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsError {
    /// File or directory not found.
    NotFound,
    /// Expected a directory but found a file.
    NotADirectory,
    /// Expected a file but found a directory.
    IsADirectory,
    /// Entry already exists.
    AlreadyExists,
    /// Bad file descriptor.
    BadFd,
    /// Permission denied.
    PermissionDenied,
    /// I/O error.
    IoError,
    /// Invalid argument.
    InvalidArgument,
    /// Operation not supported.
    NotSupported,
}

impl FsError {
    /// Convert to a POSIX errno value.
    #[must_use]
    pub fn to_errno(self) -> isize {
        match self {
            FsError::NotFound => hadron_core::syscall::ENOENT,
            FsError::NotADirectory => hadron_core::syscall::ENOTDIR,
            FsError::IsADirectory => hadron_core::syscall::EISDIR,
            FsError::AlreadyExists => hadron_core::syscall::EEXIST,
            FsError::BadFd => hadron_core::syscall::EBADF,
            FsError::PermissionDenied => hadron_core::syscall::EACCES,
            FsError::IoError => hadron_core::syscall::EIO,
            FsError::InvalidArgument => hadron_core::syscall::EINVAL,
            FsError::NotSupported => hadron_core::syscall::ENOSYS,
        }
    }
}

/// A filesystem inode -- represents a file, directory, or device.
///
/// Object-safe trait for dynamic dispatch across filesystem types.
/// Read and write return pinned boxed futures to support async I/O
/// while remaining object-safe.
pub trait Inode: Send + Sync {
    /// Returns the type of this inode.
    fn inode_type(&self) -> InodeType;

    /// Returns the size of the file data in bytes.
    fn size(&self) -> usize;

    /// Returns the permissions of this inode.
    fn permissions(&self) -> Permissions;

    /// Read data from this inode at the given offset.
    ///
    /// Returns the number of bytes read. For in-memory filesystems the
    /// returned future resolves immediately in a single poll.
    fn read<'a>(
        &'a self,
        offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>;

    /// Write data to this inode at the given offset.
    ///
    /// Returns the number of bytes written. For in-memory filesystems the
    /// returned future resolves immediately in a single poll.
    fn write<'a>(
        &'a self,
        offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>;

    /// Look up a child entry by name (directories only).
    ///
    /// # Errors
    ///
    /// Returns [`FsError::NotFound`] if the name does not exist, or
    /// [`FsError::NotADirectory`] if this inode is not a directory.
    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>, FsError>;

    /// List all entries in this directory.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::NotADirectory`] if this inode is not a directory.
    fn readdir(&self) -> Result<Vec<DirEntry>, FsError>;

    /// Create a child entry in this directory.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::AlreadyExists`] if the name already exists, or
    /// [`FsError::NotADirectory`] if this inode is not a directory.
    fn create(
        &self,
        name: &str,
        itype: InodeType,
        perms: Permissions,
    ) -> Result<Arc<dyn Inode>, FsError>;

    /// Remove a child entry from this directory.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::NotFound`] if the name does not exist.
    fn unlink(&self, name: &str) -> Result<(), FsError>;
}

/// A mounted filesystem.
pub trait FileSystem: Send + Sync {
    /// Returns the filesystem type name.
    fn name(&self) -> &'static str;

    /// Returns the root inode of this filesystem.
    fn root(&self) -> Arc<dyn Inode>;
}

/// Construct a noop waker for single-poll helpers.
fn noop_waker() -> core::task::Waker {
    use core::task::{RawWaker, RawWakerVTable, Waker};

    fn noop_clone(_: *const ()) -> RawWaker {
        noop_raw_waker()
    }
    fn noop(_: *const ()) {}
    fn noop_raw_waker() -> RawWaker {
        RawWaker::new(
            core::ptr::null(),
            &RawWakerVTable::new(noop_clone, noop, noop, noop),
        )
    }

    // SAFETY: The noop waker vtable functions are valid (they do nothing).
    unsafe { Waker::from_raw(noop_raw_waker()) }
}

/// Poll a future that is expected to resolve immediately (single poll).
///
/// Constructs a noop waker, polls once, and panics if the future returns
/// `Pending`. This is appropriate for in-memory filesystem operations
/// (ramfs, devfs) that never yield.
///
/// # Panics
///
/// Panics if the future returns `Pending`.
#[must_use]
pub fn poll_immediate<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
    use core::task::{Context, Poll};

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    match future.as_mut().poll(&mut cx) {
        Poll::Ready(val) => val,
        Poll::Pending => panic!("poll_immediate: future returned Pending"),
    }
}

/// Try to poll a future once, returning `Some(value)` if it resolves
/// immediately or `None` if it would block.
///
/// Used by syscall handlers that need to attempt synchronous I/O first
/// and fall back to the async TRAP_IO mechanism if the future is not ready.
#[must_use]
pub fn try_poll_immediate<T>(
    mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>,
) -> Option<T> {
    use core::task::{Context, Poll};

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    match future.as_mut().poll(&mut cx) {
        Poll::Ready(val) => Some(val),
        Poll::Pending => None,
    }
}
