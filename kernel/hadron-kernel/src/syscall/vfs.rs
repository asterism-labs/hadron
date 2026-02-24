//! VFS syscall handlers: open, read, write, close, stat, readdir, dup, seek,
//! mkdir, unlink, and related operations.

use crate::id::Fd;
use crate::syscall::EFAULT;
use crate::syscall::userptr::UserSlice;

use alloc::sync::Arc;

use crate::fs::file::OpenFlags;
use crate::fs::{Inode, poll_immediate, try_poll_immediate};

// ── Shared helpers ──────────────────────────────────────────────────────

/// Look up an fd and clone its inode, returning `-EBADF` on failure.
fn fd_inode(fd: Fd) -> Result<Arc<dyn Inode>, isize> {
    crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return Err(-crate::syscall::EBADF);
        };
        Ok(file.inode.clone())
    })
}

/// Look up an fd, verify `required_flags`, and return (inode, offset).
///
/// Returns `-EBADF` if the fd is invalid or the required flags are not set.
fn fd_inode_checked(fd: Fd, required_flags: OpenFlags) -> Result<(Arc<dyn Inode>, usize), isize> {
    crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return Err(-crate::syscall::EBADF);
        };
        if !file.flags.contains(required_flags) {
            return Err(-crate::syscall::EBADF);
        }
        Ok((file.inode.clone(), file.offset))
    })
}

/// Resolve `path` relative to the current working directory.
///
/// Absolute paths are returned as-is. Relative paths are joined to the
/// process CWD.
pub(super) fn resolve_cwd_path(path: &str) -> alloc::string::String {
    if path.starts_with('/') {
        return alloc::string::String::from(path);
    }
    let cwd = crate::proc::ProcessTable::with_current(|p| p.cwd.lock().clone());
    if cwd.ends_with('/') {
        alloc::format!("{cwd}{path}")
    } else {
        alloc::format!("{cwd}/{path}")
    }
}

/// `sys_vnode_open` — open a file by path, returning a file descriptor.
///
/// Arguments:
/// - `path_ptr`: user-space pointer to the path string
/// - `path_len`: length of the path string
/// - `flags`: open flags (bitwise OR of `OpenFlags` values)
///
/// Returns a non-negative fd on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_vnode_open(path_ptr: usize, path_len: usize, flags: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(path_ptr, path_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [path_ptr, path_ptr+path_len) is in user space.
    let path_bytes = unsafe { user_slice.as_slice() };
    let Ok(path) = core::str::from_utf8(path_bytes) else {
        return -crate::syscall::EINVAL;
    };

    #[expect(clippy::cast_possible_truncation, reason = "open flags fit in u32")]
    let open_flags = OpenFlags::from_bits_truncate(flags as u32);

    // Resolve path via VFS.
    let Ok(inode) = crate::fs::vfs::with_vfs(|vfs| vfs.resolve(path)) else {
        return -crate::syscall::ENOENT;
    };

    // Check if the inode wants to substitute a different inode on open
    // (e.g. /dev/ptmx allocates a new PTY master).
    let inode = match inode.on_open() {
        Ok(Some(replacement)) => replacement,
        Ok(None) => inode,
        Err(e) => return -e.to_errno(),
    };

    // Allocate fd in the current process's fd table.
    let fd = crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        fd_table.open(inode, open_flags)
    });

    fd.as_u32() as isize
}

/// `sys_vnode_read` — read from an open file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to the destination buffer
/// - `buf_len`: maximum number of bytes to read
///
/// Returns the number of bytes read on success, or a negative errno on failure.
/// If the underlying I/O would block (e.g. pipe with no data), triggers
/// TRAP_IO to handle the async read in `process_task`.
#[expect(
    clippy::cast_possible_wrap,
    reason = "byte counts are small, wrap is impossible"
)]
pub(super) fn sys_vnode_read(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_mut_slice() };

    // Extract inode and offset, then release the process lock before I/O.
    // trap_io() does a longjmp and must never run while holding a spinlock.
    let (inode, offset) = match fd_inode_checked(fd, OpenFlags::READ) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    match try_poll_immediate(inode.read(offset, buf)) {
        Some(Ok(n)) => {
            // Re-acquire to update offset.
            crate::proc::ProcessTable::with_current(|process| {
                let mut fd_table = process.fd_table.lock();
                if let Some(f) = fd_table.get_mut(fd) {
                    f.offset += n;
                }
            });
            n as isize
        }
        Some(Err(e)) => -e.to_errno(),
        None => {
            // Drop the Arc before the longjmp to avoid leaking the reference count.
            // The TRAP_IO handler re-fetches the inode from the fd table.
            drop(inode);
            // I/O would block — trap to process_task for async handling.
            // This is outside with_current_process, so the longjmp is safe.
            trap_io(fd, buf_ptr, buf_len, false)
        }
    }
}

/// `sys_vnode_write` — write to an open file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to the source buffer
/// - `buf_len`: number of bytes to write
///
/// Returns the number of bytes written on success, or a negative errno on failure.
/// If the underlying I/O would block (e.g. pipe with full buffer), triggers
/// TRAP_IO to handle the async write in `process_task`.
#[expect(
    clippy::cast_possible_wrap,
    reason = "byte counts are small, wrap is impossible"
)]
pub(super) fn sys_vnode_write(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_slice() };

    // Extract inode and offset, then release the process lock before I/O.
    // trap_io() does a longjmp and must never run while holding a spinlock.
    let (inode, offset) = match fd_inode_checked(fd, OpenFlags::WRITE) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    match try_poll_immediate(inode.write(offset, buf)) {
        Some(Ok(n)) => {
            // Re-acquire to update offset.
            crate::proc::ProcessTable::with_current(|process| {
                let mut fd_table = process.fd_table.lock();
                if let Some(f) = fd_table.get_mut(fd) {
                    f.offset += n;
                }
            });
            n as isize
        }
        Some(Err(e)) => {
            if matches!(e, crate::fs::FsError::BrokenPipe) {
                crate::proc::ProcessTable::with_current(|p| {
                    p.signals.post(crate::syscall::SIGPIPE);
                });
            }
            -e.to_errno()
        }
        None => {
            // Drop the Arc before the longjmp to avoid leaking the reference count.
            // The TRAP_IO handler re-fetches the inode from the fd table.
            drop(inode);
            // I/O would block — trap to process_task for async handling.
            // This is outside with_current_process, so the longjmp is safe.
            trap_io(fd, buf_ptr, buf_len, true)
        }
    }
}

/// `sys_handle_close` — close a file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_handle_close(fd: usize) -> isize {
    let fd = Fd::new(fd as u32);
    crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        match fd_table.close(fd) {
            Ok(()) => 0,
            Err(e) => -e.to_errno(),
        }
    })
}

/// `sys_handle_dup` — duplicate a file descriptor (dup2 semantics).
///
/// Arguments:
/// - `old_fd`: source file descriptor
/// - `new_fd`: destination file descriptor (closed silently if already open)
///
/// Returns `new_fd` on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_handle_dup(old_fd: usize, new_fd: usize) -> isize {
    let old_fd = Fd::new(old_fd as u32);
    let new_fd = Fd::new(new_fd as u32);
    crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        let Some(src) = fd_table.get(old_fd) else {
            return -crate::syscall::EBADF;
        };
        let inode = src.inode.clone();
        let flags = src.flags;

        // If new_fd is already open, close it silently (POSIX dup2 semantics).
        let _ = fd_table.close(new_fd);

        fd_table.insert_at(new_fd, inode, flags);
        new_fd.as_u32() as isize
    })
}

/// `sys_vnode_stat` — get file status information.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to write [`StatInfo`] to
/// - `buf_len`: size of the user buffer (must be >= `size_of::<StatInfo>()`)
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_vnode_stat(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    use crate::syscall::{EINVAL, StatInfo};

    let stat_size = core::mem::size_of::<StatInfo>();
    if buf_len < stat_size {
        return -EINVAL;
    }

    let Ok(user_slice) = UserSlice::new(buf_ptr, stat_size) else {
        return -EFAULT;
    };

    let inode = match fd_inode(fd) {
        Ok(i) => i,
        Err(e) => return e,
    };

    let inode_type = match inode.inode_type() {
        crate::fs::InodeType::File => crate::syscall::INODE_TYPE_FILE,
        crate::fs::InodeType::Directory => crate::syscall::INODE_TYPE_DIR,
        crate::fs::InodeType::CharDevice => crate::syscall::INODE_TYPE_CHARDEV,
        crate::fs::InodeType::Symlink => crate::syscall::INODE_TYPE_SYMLINK,
    };

    let perms = inode.permissions();
    let permissions: u32 =
        u32::from(perms.read) | (u32::from(perms.write) << 1) | (u32::from(perms.execute) << 2);

    let info = StatInfo {
        inode_type,
        _pad: [0; 3],
        size: inode.size() as u64,
        permissions,
    };

    // SAFETY: UserSlice validated the pointer range is in user space,
    // and we write exactly stat_size bytes.
    let out = unsafe { user_slice.as_mut_slice() };
    // SAFETY: StatInfo is repr(C) and contains only scalar fields.
    let info_bytes =
        unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(info).cast::<u8>(), stat_size) };
    out[..stat_size].copy_from_slice(info_bytes);
    0
}

/// `sys_handle_pipe` — create a pipe and return [read_fd, write_fd].
///
/// Arguments:
/// - `fds_ptr`: user-space pointer to write two `usize` values (read_fd, write_fd)
///
/// Returns 0 on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_handle_pipe(fds_ptr: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(fds_ptr, 2 * core::mem::size_of::<usize>()) else {
        return -EFAULT;
    };

    let (reader, writer) = crate::ipc::pipe::pipe();

    let (read_fd, write_fd) = crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        let rfd = fd_table.open(reader, OpenFlags::READ);
        let wfd = fd_table.open(writer, OpenFlags::WRITE);
        (rfd, wfd)
    });

    // SAFETY: UserSlice validated the pointer range is in user space.
    // The ABI returns fd numbers as usize values to userspace.
    unsafe {
        let dst = user_slice.addr() as *mut usize;
        core::ptr::write(dst, read_fd.as_usize());
        core::ptr::write(dst.add(1), write_fd.as_usize());
    }

    0
}

/// `sys_vnode_readdir` — read directory entries.
///
/// Arguments:
/// - `fd`: file descriptor number (must refer to a directory)
/// - `buf_ptr`: user-space pointer to write [`DirEntryInfo`] array to
/// - `buf_len`: size of the user buffer in bytes
///
/// Returns the number of entries written on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "entry counts are small, wrap is impossible"
)]
pub(super) fn sys_vnode_readdir(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    use crate::syscall::DirEntryInfo;

    let entry_size = core::mem::size_of::<DirEntryInfo>();
    let max_entries = buf_len / entry_size;
    if max_entries == 0 {
        return -crate::syscall::EINVAL;
    }

    let total_bytes = max_entries * entry_size;
    let Ok(user_slice) = UserSlice::new(buf_ptr, total_bytes) else {
        return -EFAULT;
    };

    let inode = match fd_inode(fd) {
        Ok(i) => i,
        Err(e) => return e,
    };

    let entries = match poll_immediate(inode.readdir()) {
        Ok(entries) => entries,
        Err(e) => return -e.to_errno(),
    };

    // SAFETY: UserSlice validated the pointer range is in user space.
    let out = unsafe { user_slice.as_mut_slice() };
    let mut written = 0;

    for entry in &entries {
        if written >= max_entries {
            break;
        }

        let inode_type = match entry.inode_type {
            crate::fs::InodeType::File => crate::syscall::INODE_TYPE_FILE,
            crate::fs::InodeType::Directory => crate::syscall::INODE_TYPE_DIR,
            crate::fs::InodeType::CharDevice => crate::syscall::INODE_TYPE_CHARDEV,
            crate::fs::InodeType::Symlink => crate::syscall::INODE_TYPE_SYMLINK,
        };

        let name_bytes = entry.name.as_bytes();
        let name_len = name_bytes.len().min(60);

        let mut info = DirEntryInfo {
            inode_type,
            name_len: name_len as u8,
            _pad: [0; 2],
            name: [0; 60],
        };
        info.name[..name_len].copy_from_slice(&name_bytes[..name_len]);

        // SAFETY: DirEntryInfo is repr(C) and contains only scalar fields.
        let info_bytes = unsafe {
            core::slice::from_raw_parts(core::ptr::addr_of!(info).cast::<u8>(), entry_size)
        };
        let offset = written * entry_size;
        out[offset..offset + entry_size].copy_from_slice(info_bytes);
        written += 1;
    }

    written as isize
}

/// `sys_vnode_unlink` — remove a file or empty directory by path.
///
/// Arguments:
/// - `path_ptr`: user-space pointer to the path string
/// - `path_len`: length of the path string
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_vnode_unlink(path_ptr: usize, path_len: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(path_ptr, path_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [path_ptr, path_ptr+path_len) is in user space.
    let path_bytes = unsafe { user_slice.as_slice() };
    let Ok(path) = core::str::from_utf8(path_bytes) else {
        return -crate::syscall::EINVAL;
    };

    // Split into parent directory path and entry name.
    let (parent_path, name) = match path.rsplit_once('/') {
        Some((parent, name)) => {
            let parent = if parent.is_empty() { "/" } else { parent };
            (parent, name)
        }
        None => return -crate::syscall::EINVAL,
    };

    if name.is_empty() {
        return -crate::syscall::EINVAL;
    }

    // Resolve the parent directory.
    let parent_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(parent_path)) {
        Ok(inode) => inode,
        Err(e) => return -e.to_errno(),
    };

    // Call unlink on the parent.
    match crate::fs::poll_immediate(parent_inode.unlink(name)) {
        Ok(()) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_vnode_seek` — reposition the file offset of an open fd.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `offset`: offset value (interpretation depends on `whence`)
/// - `whence`: `SEEK_SET` (0), `SEEK_CUR` (1), or `SEEK_END` (2)
///
/// Returns the new absolute offset on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "file offsets are small, wrap is impossible"
)]
pub(super) fn sys_vnode_seek(fd: usize, offset: usize, whence: usize) -> isize {
    use crate::syscall::{EINVAL, ESPIPE, SEEK_CUR, SEEK_END, SEEK_SET};

    let fd = Fd::new(fd as u32);
    let offset = offset as isize;

    crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get_mut(fd) else {
            return -crate::syscall::EBADF;
        };

        // Pipes and character devices are not seekable.
        if matches!(file.inode.inode_type(), crate::fs::InodeType::CharDevice) {
            return -ESPIPE;
        }

        let file_size = file.inode.size();
        let current = file.offset as isize;

        let new_offset = match whence {
            SEEK_SET => offset,
            SEEK_CUR => current.saturating_add(offset),
            SEEK_END => (file_size as isize).saturating_add(offset),
            _ => return -EINVAL,
        };

        if new_offset < 0 {
            return -EINVAL;
        }

        file.offset = new_offset as usize;
        new_offset as isize
    })
}

/// `sys_vnode_mkdir` — create a directory.
///
/// Arguments:
/// - `path_ptr`: user-space pointer to the path string
/// - `path_len`: length of the path string
/// - `permissions`: permission bitmask (bit 0=read, 1=write, 2=exec)
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_vnode_mkdir(path_ptr: usize, path_len: usize, permissions: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(path_ptr, path_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [path_ptr, path_ptr+path_len) is in user space.
    let path_bytes = unsafe { user_slice.as_slice() };
    let Ok(path) = core::str::from_utf8(path_bytes) else {
        return -crate::syscall::EINVAL;
    };

    // Split into parent directory path and new directory name.
    let (parent_path, name) = match path.rsplit_once('/') {
        Some((parent, name)) => {
            let parent = if parent.is_empty() { "/" } else { parent };
            (parent, name)
        }
        None => return -crate::syscall::EINVAL,
    };

    if name.is_empty() {
        return -crate::syscall::EINVAL;
    }

    let perms = crate::fs::Permissions {
        read: permissions & 0x1 != 0,
        write: permissions & 0x2 != 0,
        execute: permissions & 0x4 != 0,
    };

    // Resolve the parent directory.
    let parent_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(parent_path)) {
        Ok(inode) => inode,
        Err(e) => return -e.to_errno(),
    };

    // Create the directory entry.
    match crate::fs::poll_immediate(parent_inode.create(
        name,
        crate::fs::InodeType::Directory,
        perms,
    )) {
        Ok(_inode) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_handle_dup_lowest` — duplicate a file descriptor to the lowest free fd.
///
/// Arguments:
/// - `old_fd`: source file descriptor
///
/// Returns the new fd on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_handle_dup_lowest(old_fd: usize) -> isize {
    let old_fd = Fd::new(old_fd as u32);
    crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        match fd_table.dup_lowest(old_fd) {
            Some(new_fd) => new_fd.as_u32() as isize,
            None => -crate::syscall::EBADF,
        }
    })
}

/// `sys_handle_fcntl` — perform fcntl operations on a file descriptor.
///
/// Commands:
/// - `F_DUPFD(arg)`: duplicate to lowest free fd >= arg
/// - `F_DUPFD_CLOEXEC(arg)`: same, with CLOEXEC set
/// - `F_GETFD`: return `FD_CLOEXEC` if CLOEXEC is set, else 0
/// - `F_SETFD(arg)`: set/clear CLOEXEC based on `arg & FD_CLOEXEC`
/// - `F_GETFL`: return file status flags (APPEND, NONBLOCK, READ, WRITE)
/// - `F_SETFL(arg)`: set modifiable flags (APPEND, NONBLOCK)
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers and flag values are small, wrap is impossible"
)]
pub(super) fn sys_handle_fcntl(fd: usize, cmd: usize, arg: usize) -> isize {
    use crate::syscall::{
        EINVAL, F_DUPFD, F_DUPFD_CLOEXEC, F_GETFD, F_GETFL, F_SETFD, F_SETFL, FD_CLOEXEC,
    };

    let fd = Fd::new(fd as u32);

    match cmd {
        F_DUPFD => crate::proc::ProcessTable::with_current(|process| {
            let mut fd_table = process.fd_table.lock();
            match fd_table.dup_lowest_from(fd, Fd::new(arg as u32), OpenFlags::empty()) {
                Some(new_fd) => new_fd.as_u32() as isize,
                None => -crate::syscall::EBADF,
            }
        }),
        F_DUPFD_CLOEXEC => crate::proc::ProcessTable::with_current(|process| {
            let mut fd_table = process.fd_table.lock();
            match fd_table.dup_lowest_from(fd, Fd::new(arg as u32), OpenFlags::CLOEXEC) {
                Some(new_fd) => new_fd.as_u32() as isize,
                None => -crate::syscall::EBADF,
            }
        }),
        F_GETFD => crate::proc::ProcessTable::with_current(|process| {
            let fd_table = process.fd_table.lock();
            match fd_table.get(fd) {
                Some(f) => {
                    if f.flags.contains(OpenFlags::CLOEXEC) {
                        FD_CLOEXEC as isize
                    } else {
                        0
                    }
                }
                None => -crate::syscall::EBADF,
            }
        }),
        F_SETFD => crate::proc::ProcessTable::with_current(|process| {
            let mut fd_table = process.fd_table.lock();
            match fd_table.get_mut(fd) {
                Some(f) => {
                    if arg & FD_CLOEXEC != 0 {
                        f.flags |= OpenFlags::CLOEXEC;
                    } else {
                        f.flags -= OpenFlags::CLOEXEC;
                    }
                    0
                }
                None => -crate::syscall::EBADF,
            }
        }),
        F_GETFL => crate::proc::ProcessTable::with_current(|process| {
            let fd_table = process.fd_table.lock();
            match fd_table.get(fd) {
                Some(f) => f.flags.bits() as isize,
                None => -crate::syscall::EBADF,
            }
        }),
        F_SETFL => {
            // Only APPEND and NONBLOCK are modifiable via F_SETFL.
            let modifiable = OpenFlags::APPEND | OpenFlags::NONBLOCK;
            #[expect(clippy::cast_possible_truncation, reason = "flags fit in u32")]
            let new_bits = OpenFlags::from_bits_truncate(arg as u32) & modifiable;

            crate::proc::ProcessTable::with_current(|process| {
                let mut fd_table = process.fd_table.lock();
                match fd_table.get_mut(fd) {
                    Some(f) => {
                        f.flags = (f.flags - modifiable) | new_bits;
                        0
                    }
                    None => -crate::syscall::EBADF,
                }
            })
        }
        _ => -EINVAL,
    }
}

/// `sys_handle_pipe2` — create a pipe with flags.
///
/// Arguments:
/// - `fds_ptr`: user-space pointer to write two `usize` values (read_fd, write_fd)
/// - `flags`: `PIPE_CLOEXEC` and/or `PIPE_NONBLOCK`
///
/// Returns 0 on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_handle_pipe2(fds_ptr: usize, flags: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(fds_ptr, 2 * core::mem::size_of::<usize>()) else {
        return -EFAULT;
    };

    let (reader, writer) = crate::ipc::pipe::pipe();

    let mut read_flags = OpenFlags::READ;
    let mut write_flags = OpenFlags::WRITE;

    if flags & crate::syscall::PIPE_CLOEXEC != 0 {
        read_flags |= OpenFlags::CLOEXEC;
        write_flags |= OpenFlags::CLOEXEC;
    }
    if flags & crate::syscall::PIPE_NONBLOCK != 0 {
        read_flags |= OpenFlags::NONBLOCK;
        write_flags |= OpenFlags::NONBLOCK;
    }

    let (read_fd, write_fd) = crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        let rfd = fd_table.open(reader, read_flags);
        let wfd = fd_table.open(writer, write_flags);
        (rfd, wfd)
    });

    // SAFETY: UserSlice validated the pointer range is in user space.
    unsafe {
        let dst = user_slice.addr() as *mut usize;
        core::ptr::write(dst, read_fd.as_usize());
        core::ptr::write(dst.add(1), write_fd.as_usize());
    }

    0
}

/// `sys_vnode_rename` — rename (move) a file or directory.
pub(super) fn sys_vnode_rename(
    old_ptr: usize,
    old_len: usize,
    new_ptr: usize,
    new_len: usize,
) -> isize {
    let Ok(old_slice) = UserSlice::new(old_ptr, old_len) else {
        return -EFAULT;
    };
    let Ok(new_slice) = UserSlice::new(new_ptr, new_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated ranges.
    let old_bytes = unsafe { old_slice.as_slice() };
    let new_bytes = unsafe { new_slice.as_slice() };

    let Ok(old_path) = core::str::from_utf8(old_bytes) else {
        return -crate::syscall::EINVAL;
    };
    let Ok(new_path) = core::str::from_utf8(new_bytes) else {
        return -crate::syscall::EINVAL;
    };

    // Split old path into parent + name.
    let (old_parent, old_name) = match old_path.rsplit_once('/') {
        Some((p, n)) => (if p.is_empty() { "/" } else { p }, n),
        None => return -crate::syscall::EINVAL,
    };

    // Split new path into parent + name.
    let (new_parent, new_name) = match new_path.rsplit_once('/') {
        Some((p, n)) => (if p.is_empty() { "/" } else { p }, n),
        None => return -crate::syscall::EINVAL,
    };

    if old_name.is_empty() || new_name.is_empty() {
        return -crate::syscall::EINVAL;
    }

    let old_parent_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(old_parent)) {
        Ok(i) => i,
        Err(e) => return -e.to_errno(),
    };

    let new_parent_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(new_parent)) {
        Ok(i) => i,
        Err(e) => return -e.to_errno(),
    };

    match crate::fs::poll_immediate(old_parent_inode.rename(old_name, &*new_parent_inode, new_name))
    {
        Ok(()) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_vnode_symlink` — create a symbolic link.
pub(super) fn sys_vnode_symlink(
    target_ptr: usize,
    target_len: usize,
    link_ptr: usize,
    link_len: usize,
) -> isize {
    let Ok(target_slice) = UserSlice::new(target_ptr, target_len) else {
        return -EFAULT;
    };
    let Ok(link_slice) = UserSlice::new(link_ptr, link_len) else {
        return -EFAULT;
    };

    let target_bytes = unsafe { target_slice.as_slice() };
    let link_bytes = unsafe { link_slice.as_slice() };

    let Ok(target) = core::str::from_utf8(target_bytes) else {
        return -crate::syscall::EINVAL;
    };
    let Ok(link_path) = core::str::from_utf8(link_bytes) else {
        return -crate::syscall::EINVAL;
    };

    // Split link path into parent + name.
    let (parent_path, name) = match link_path.rsplit_once('/') {
        Some((p, n)) => (if p.is_empty() { "/" } else { p }, n),
        None => return -crate::syscall::EINVAL,
    };

    if name.is_empty() {
        return -crate::syscall::EINVAL;
    }

    let parent_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(parent_path)) {
        Ok(i) => i,
        Err(e) => return -e.to_errno(),
    };

    let perms = crate::fs::Permissions {
        read: true,
        write: true,
        execute: true,
    };

    match parent_inode.create_symlink(name, target, perms) {
        Ok(_) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_vnode_link` — create a hard link.
pub(super) fn sys_vnode_link(
    target_ptr: usize,
    target_len: usize,
    link_ptr: usize,
    link_len: usize,
) -> isize {
    let Ok(target_slice) = UserSlice::new(target_ptr, target_len) else {
        return -EFAULT;
    };
    let Ok(link_slice) = UserSlice::new(link_ptr, link_len) else {
        return -EFAULT;
    };

    let target_bytes = unsafe { target_slice.as_slice() };
    let link_bytes = unsafe { link_slice.as_slice() };

    let Ok(target_path) = core::str::from_utf8(target_bytes) else {
        return -crate::syscall::EINVAL;
    };
    let Ok(link_path) = core::str::from_utf8(link_bytes) else {
        return -crate::syscall::EINVAL;
    };

    // Resolve the target inode.
    let target_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(target_path)) {
        Ok(i) => i,
        Err(e) => return -e.to_errno(),
    };

    // Split link path into parent + name.
    let (parent_path, name) = match link_path.rsplit_once('/') {
        Some((p, n)) => (if p.is_empty() { "/" } else { p }, n),
        None => return -crate::syscall::EINVAL,
    };

    if name.is_empty() {
        return -crate::syscall::EINVAL;
    }

    let parent_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(parent_path)) {
        Ok(i) => i,
        Err(e) => return -e.to_errno(),
    };

    match crate::fs::poll_immediate(parent_inode.link(name, &*target_inode)) {
        Ok(()) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_vnode_readlink` — read the target of a symbolic link.
#[expect(
    clippy::cast_possible_wrap,
    reason = "link target lengths are small, wrap is impossible"
)]
pub(super) fn sys_vnode_readlink(
    path_ptr: usize,
    path_len: usize,
    buf_ptr: usize,
    buf_len: usize,
) -> isize {
    let Ok(path_slice) = UserSlice::new(path_ptr, path_len) else {
        return -EFAULT;
    };
    let Ok(buf_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    let path_bytes = unsafe { path_slice.as_slice() };
    let Ok(path) = core::str::from_utf8(path_bytes) else {
        return -crate::syscall::EINVAL;
    };

    // Resolve without following the final symlink — we need the symlink itself.
    // For now, resolve normally (VFS follows symlinks). We look up the parent
    // and then call read_link on the child name.
    let (parent_path, name) = match path.rsplit_once('/') {
        Some((p, n)) => (if p.is_empty() { "/" } else { p }, n),
        None => return -crate::syscall::EINVAL,
    };

    if name.is_empty() {
        return -crate::syscall::EINVAL;
    }

    let parent_inode = match crate::fs::vfs::with_vfs(|vfs| vfs.resolve(parent_path)) {
        Ok(i) => i,
        Err(e) => return -e.to_errno(),
    };

    // Lookup the child without following symlinks.
    let child = match crate::fs::poll_immediate(parent_inode.lookup(name)) {
        Ok(i) => i,
        Err(e) => return -e.to_errno(),
    };

    let target = match child.read_link() {
        Ok(t) => t,
        Err(e) => return -e.to_errno(),
    };

    let target_bytes = target.as_bytes();
    if target_bytes.len() > buf_len {
        return -crate::syscall::EINVAL;
    }

    let buf = unsafe { buf_slice.as_mut_slice() };
    buf[..target_bytes.len()].copy_from_slice(target_bytes);
    target_bytes.len() as isize
}

/// `sys_vnode_truncate` — truncate a file to a specified length.
pub(super) fn sys_vnode_truncate(fd: usize, len: usize) -> isize {
    let fd = Fd::new(fd as u32);

    let (inode, _) = match fd_inode_checked(fd, OpenFlags::WRITE) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    match crate::fs::poll_immediate(inode.truncate(len)) {
        Ok(()) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_vnode_fstatat` — stat a file relative to a directory fd.
///
/// `dirfd` is `AT_FDCWD` (0xFFFF_FF9C) for CWD or an open directory fd.
/// `path_ptr`/`path_len` is the relative or absolute path.
/// `buf` receives a [`StatInfo`]. `flags` is a bitmask of `AT_SYMLINK_NOFOLLOW`.
pub(super) fn sys_vnode_fstatat(
    dirfd: usize,
    path_ptr: usize,
    path_len: usize,
    buf: usize,
    _flags: usize,
) -> isize {
    use crate::fs::vfs;
    use crate::syscall::{EINVAL, StatInfo};
    use hadron_syscall::AT_FDCWD;

    let stat_size = core::mem::size_of::<StatInfo>();

    // Read path from user memory.
    let path_slice = match UserSlice::new(path_ptr, path_len) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let path_bytes =
        unsafe { core::slice::from_raw_parts(path_slice.addr() as *const u8, path_len) };
    let path = match core::str::from_utf8(path_bytes) {
        Ok(p) => p,
        Err(_) => return -EINVAL,
    };

    // Resolve the path to an inode.
    let inode = if path.starts_with('/') || dirfd == AT_FDCWD {
        let resolved_path = resolve_cwd_path(path);
        match vfs::resolve(&resolved_path) {
            Ok(i) => i,
            Err(e) => return -e.to_errno(),
        }
    } else {
        // Resolve relative to the directory fd.
        let dir_fd = Fd::new(dirfd as u32);
        let dir_inode = match fd_inode(dir_fd) {
            Ok(i) => i,
            Err(e) => return e,
        };
        if dir_inode.inode_type() != crate::fs::InodeType::Directory {
            return -crate::syscall::ENOTDIR;
        }
        match vfs::resolve_relative(dir_inode, path) {
            Ok(i) => i,
            Err(e) => return -e.to_errno(),
        }
    };

    let Ok(user_slice) = UserSlice::new(buf, stat_size) else {
        return -EFAULT;
    };

    let inode_type = match inode.inode_type() {
        crate::fs::InodeType::File => crate::syscall::INODE_TYPE_FILE,
        crate::fs::InodeType::Directory => crate::syscall::INODE_TYPE_DIR,
        crate::fs::InodeType::CharDevice => crate::syscall::INODE_TYPE_CHARDEV,
        crate::fs::InodeType::Symlink => crate::syscall::INODE_TYPE_SYMLINK,
    };

    let perms = inode.permissions();
    let permissions: u32 =
        u32::from(perms.read) | (u32::from(perms.write) << 1) | (u32::from(perms.execute) << 2);

    let info = StatInfo {
        inode_type,
        _pad: [0; 3],
        size: inode.size() as u64,
        permissions,
    };

    let out = unsafe { user_slice.as_mut_slice() };
    let info_bytes =
        unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(info).cast::<u8>(), stat_size) };
    out[..stat_size].copy_from_slice(info_bytes);
    0
}

/// `sys_handle_tcsetpgrp` — set the foreground process group of a TTY.
///
/// Currently operates on the active TTY regardless of `fd`, since all
/// terminal fds (/dev/console, /dev/tty0) point to the same TTY.
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_handle_tcsetpgrp(_fd: usize, pgid: usize) -> isize {
    let pgid = pgid as u32;
    crate::tty::active_tty().set_foreground_pgid(pgid);
    0
}

/// `sys_handle_tcgetpgrp` — get the foreground process group of a TTY.
///
/// Currently operates on the active TTY regardless of `fd`.
///
/// Returns the PGID on success (0 if none set), or a negative errno.
#[expect(
    clippy::cast_possible_wrap,
    reason = "PGIDs are small positive integers, wrap is impossible"
)]
pub(super) fn sys_handle_tcgetpgrp(_fd: usize) -> isize {
    crate::tty::active_tty().foreground_pgid().unwrap_or(0) as isize
}

/// Trigger a TRAP_IO longjmp back to `process_task` for async I/O.
///
/// Sets the I/O parameters, restores kernel CR3 and GS bases, then
/// calls `restore_kernel_context` — never returns.
pub(super) fn trap_io(fd: Fd, buf_ptr: usize, buf_len: usize, is_write: bool) -> ! {
    use crate::arch::x86_64::registers::control::Cr3;
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
    use crate::arch::x86_64::userspace::restore_kernel_context;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();

    // SAFETY: Restoring kernel CR3 and GS bases is the standard pattern
    // for returning from userspace context to kernel context.
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    crate::proc::IoState::set_params(fd, buf_ptr, buf_len, is_write);
    crate::proc::TrapContext::set_trap_reason(crate::proc::TrapReason::Io);

    let saved_rsp = crate::proc::TrapContext::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}
