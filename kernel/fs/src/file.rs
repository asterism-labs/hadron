//! File descriptors and file descriptor tables.
//!
//! Each process has a [`FileDescriptorTable`] mapping [`Fd`] numbers
//! to open [`FileDescriptor`]s. File descriptors hold a reference to an
//! [`Inode`](super::Inode) plus an offset and flags.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;

use bitflags::bitflags;

use super::{FsError, Inode};
use hadron_core::id::Fd;

bitflags! {
    /// Flags for opening a file.
    #[derive(Debug, Clone, Copy)]
    pub struct OpenFlags: u32 {
        /// Open for reading.
        const READ      = 0x0001;
        /// Open for writing.
        const WRITE     = 0x0002;
        /// Create the file if it does not exist.
        const CREATE    = 0x0004;
        /// Truncate the file to zero length on open.
        const TRUNCATE  = 0x0008;
        /// Writes always append to end of file.
        const APPEND    = 0x0010;
        /// Close this fd on `execve`.
        const CLOEXEC   = 0x0020;
        /// Non-blocking I/O mode.
        const NONBLOCK  = 0x0040;
        /// Fail if the path does not refer to a directory.
        const DIRECTORY = 0x0080;
        /// With `CREATE`, fail if the file already exists.
        const EXCL      = 0x0100;
    }
}

/// An open file descriptor.
#[derive(Clone)]
pub struct FileDescriptor {
    /// The inode backing this fd.
    pub inode: Arc<dyn Inode>,
    /// Current read/write offset.
    pub offset: usize,
    /// Open flags.
    pub flags: OpenFlags,
}

/// Per-process file descriptor table.
#[derive(Clone)]
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

    /// Close all file descriptors that have `CLOEXEC` set.
    /// Called during `execve` to prevent fd leaks.
    pub fn close_cloexec(&mut self) {
        self.fds
            .retain(|_, desc| !desc.flags.contains(OpenFlags::CLOEXEC));
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

    /// Duplicate a file descriptor to the lowest available fd number.
    ///
    /// Returns the newly allocated fd, or `None` if `src_fd` is not open.
    pub fn dup_lowest(&mut self, src_fd: Fd) -> Option<Fd> {
        self.dup_lowest_from(src_fd, Fd::new(0), OpenFlags::empty())
    }

    /// Duplicate a file descriptor to the lowest available fd >= `min_fd`.
    ///
    /// `extra_flags` are OR'd onto the new fd's flags (e.g. `CLOEXEC`).
    /// Returns the newly allocated fd, or `None` if `src_fd` is not open.
    pub fn dup_lowest_from(
        &mut self,
        src_fd: Fd,
        min_fd: Fd,
        extra_flags: OpenFlags,
    ) -> Option<Fd> {
        let src = self.fds.get(&src_fd)?;
        let inode = src.inode.clone();
        let offset = src.offset;
        let flags = src.flags | extra_flags;

        // Find the lowest unused fd number starting from min_fd.
        let mut candidate = min_fd;
        while self.fds.contains_key(&candidate) {
            candidate = Fd::new(candidate.as_u32() + 1);
        }

        self.fds.insert(
            candidate,
            FileDescriptor {
                inode,
                offset,
                flags,
            },
        );

        if candidate >= self.next_fd {
            self.next_fd = Fd::new(candidate.as_u32() + 1);
        }

        Some(candidate)
    }
}
