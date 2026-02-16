//! VFS syscall handlers: open, read, write.

use hadron_core::syscall::userptr::UserSlice;
use hadron_core::syscall::EFAULT;

use crate::fs::file::OpenFlags;
use crate::fs::poll_immediate;

/// `sys_vnode_open` — open a file by path, returning a file descriptor.
///
/// Arguments:
/// - `path_ptr`: user-space pointer to the path string
/// - `path_len`: length of the path string
/// - `flags`: open flags (bitwise OR of `OpenFlags` values)
///
/// Returns a non-negative fd on success, or a negative errno on failure.
#[allow(clippy::cast_possible_wrap)] // fd numbers are small, wrap is impossible
pub(super) fn sys_vnode_open(path_ptr: usize, path_len: usize, flags: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(path_ptr, path_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [path_ptr, path_ptr+path_len) is in user space.
    let path_bytes = unsafe { user_slice.as_slice() };
    let Ok(path) = core::str::from_utf8(path_bytes) else {
        return -hadron_core::syscall::EINVAL;
    };

    #[allow(clippy::cast_possible_truncation)] // flags fit in u32
    let open_flags = OpenFlags::from_bits_truncate(flags as u32);

    // Resolve path via VFS.
    let Ok(inode) = crate::fs::vfs::with_vfs(|vfs| vfs.resolve(path)) else {
        return -hadron_core::syscall::ENOENT;
    };

    // Allocate fd in the current process's fd table.
    let fd = crate::proc::with_current_process(|process| {
        let mut fd_table = process.fd_table.lock();
        fd_table.open(inode, open_flags)
    });

    fd as isize
}

/// `sys_vnode_read` — read from an open file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to the destination buffer
/// - `buf_len`: maximum number of bytes to read
///
/// Returns the number of bytes read on success, or a negative errno on failure.
#[allow(clippy::cast_possible_wrap)] // read byte counts are small, wrap is impossible
pub(super) fn sys_vnode_read(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_mut_slice() };

    crate::proc::with_current_process(|process| {
        let mut fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get_mut(fd) else {
            return -hadron_core::syscall::EBADF;
        };

        if !file.flags.contains(OpenFlags::READ) {
            return -hadron_core::syscall::EBADF;
        }

        let offset = file.offset;
        let inode = file.inode.clone();
        // Drop the fd_table lock before performing I/O.
        drop(fd_table);

        match poll_immediate(inode.read(offset, buf)) {
            Ok(n) => {
                // Re-acquire to update offset.
                let mut fd_table = process.fd_table.lock();
                if let Some(f) = fd_table.get_mut(fd) {
                    f.offset += n;
                }
                n as isize
            }
            Err(e) => -e.to_errno(),
        }
    })
}

/// `sys_vnode_write` — write to an open file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to the source buffer
/// - `buf_len`: number of bytes to write
///
/// Returns the number of bytes written on success, or a negative errno on failure.
#[allow(clippy::cast_possible_wrap)] // write byte counts are small, wrap is impossible
pub(super) fn sys_vnode_write(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_slice() };

    crate::proc::with_current_process(|process| {
        let mut fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get_mut(fd) else {
            return -hadron_core::syscall::EBADF;
        };

        if !file.flags.contains(OpenFlags::WRITE) {
            return -hadron_core::syscall::EBADF;
        }

        let offset = file.offset;
        let inode = file.inode.clone();
        // Drop the fd_table lock before performing I/O.
        drop(fd_table);

        match poll_immediate(inode.write(offset, buf)) {
            Ok(n) => {
                // Re-acquire to update offset.
                let mut fd_table = process.fd_table.lock();
                if let Some(f) = fd_table.get_mut(fd) {
                    f.offset += n;
                }
                n as isize
            }
            Err(e) => -e.to_errno(),
        }
    })
}
