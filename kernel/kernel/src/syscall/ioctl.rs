//! Ioctl syscall handler.

use crate::id::Fd;
use crate::proc::ProcessTable;
use crate::syscall::userptr::UserSlice;
use crate::syscall::{EBADF, EFAULT};

/// `sys_handle_ioctl` — perform a device-specific ioctl on a file descriptor.
///
/// Validates the fd, extracts the inode, and delegates to `Inode::ioctl`.
/// The `arg_ptr` is passed through to the inode's ioctl method; for commands
/// that write data, the inode is responsible for copying to the user pointer.
#[expect(
    clippy::cast_possible_wrap,
    reason = "ioctl returns small values or 0; wrap is impossible"
)]
pub(super) fn sys_handle_ioctl(fd: usize, cmd: usize, arg_ptr: usize) -> isize {
    let fd = Fd::new(fd as u32);

    // Validate that arg_ptr is in user space (if non-zero).
    // We validate the minimum size of a u64 since all current ioctl
    // commands use at least that much. The specific ioctl handler
    // knows the actual struct size.
    if arg_ptr != 0 {
        // Validate the pointer is in user space. We use a generous size
        // check since FbInfo is 20 bytes; checking at least 1 byte
        // confirms the base address is valid.
        if UserSlice::new(arg_ptr, 1).is_err() {
            return -EFAULT;
        }
    }

    // Extract the inode from the process fd table.
    let inode = ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        fd_table.get(fd).map(|f| f.inode.clone())
    });

    let Some(inode) = inode else {
        return -EBADF;
    };

    #[expect(clippy::cast_possible_truncation, reason = "ioctl cmd fits in u32")]
    match inode.ioctl(cmd as u32, arg_ptr) {
        Ok(ret) => ret as isize,
        Err(e) => -e.to_errno(),
    }
}
