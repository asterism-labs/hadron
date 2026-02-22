//! File descriptors and file descriptor tables.
//!
//! Each process has a [`FileDescriptorTable`] mapping [`Fd`] numbers
//! to open [`FileDescriptor`]s. File descriptors hold a reference to an
//! [`Inode`](super::Inode) plus an offset and flags.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;

use bitflags::bitflags;

use crate::id::Fd;
use super::{FsError, Inode};

bitflags! {
    /// Flags for opening a file.
    #[derive(Debug, Clone, Copy)]
    pub struct OpenFlags: u32 {
        /// Open for reading.
        const READ    = 0b0001;
        /// Open for writing.
        const WRITE   = 0b0010;
        /// Create the file if it does not exist.
        const CREATE  = 0b0100;
        /// Truncate the file to zero length on open.
        const TRUNCATE = 0b1000;
    }
}

/// An open file descriptor.
pub struct FileDescriptor {
    /// The inode backing this fd.
    pub inode: Arc<dyn Inode>,
    /// Current read/write offset.
    pub offset: usize,
    /// Open flags.
    pub flags: OpenFlags,
}

/// Per-process file descriptor table.
pub struct FileDescriptorTable {
    /// Open file descriptors.
    fds: BTreeMap<Fd, FileDescriptor>,
    /// Next fd number to allocate.
    next_fd: Fd,
}

impl Default for FileDescriptorTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FileDescriptorTable {
    /// Creates a new empty file descriptor table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fds: BTreeMap::new(),
            next_fd: Fd::new(0),
        }
    }

    /// Open a file, allocating the next available fd number.
    ///
    /// Returns the newly assigned fd number.
    pub fn open(&mut self, inode: Arc<dyn Inode>, flags: OpenFlags) -> Fd {
        let fd = self.next_fd;
        self.fds.insert(
            fd,
            FileDescriptor {
                inode,
                offset: 0,
                flags,
            },
        );
        self.next_fd = Fd::new(fd.as_u32() + 1);
        fd
    }

    /// Insert a file descriptor at a specific fd number.
    ///
    /// Used for setting up stdin (0), stdout (1), stderr (2).
    pub fn insert_at(&mut self, fd: Fd, inode: Arc<dyn Inode>, flags: OpenFlags) {
        self.fds.insert(
            fd,
            FileDescriptor {
                inode,
                offset: 0,
                flags,
            },
        );
        if fd >= self.next_fd {
            self.next_fd = Fd::new(fd.as_u32() + 1);
        }
    }

    /// Close a file descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`FsError::BadFd`] if `fd` is not open.
    pub fn close(&mut self, fd: Fd) -> Result<(), FsError> {
        self.fds.remove(&fd).ok_or(FsError::BadFd)?;
        Ok(())
    }

    /// Get a shared reference to a file descriptor.
    #[must_use]
    pub fn get(&self, fd: Fd) -> Option<&FileDescriptor> {
        self.fds.get(&fd)
    }

    /// Get a mutable reference to a file descriptor.
    pub fn get_mut(&mut self, fd: Fd) -> Option<&mut FileDescriptor> {
        self.fds.get_mut(&fd)
    }
}
