//! Channel IPC syscall handlers.
//!
//! Implements bidirectional message-oriented channels for inter-process
//! communication. Each channel has two endpoints; messages sent on one
//! endpoint are received on the other.

use crate::fs::try_poll_immediate;
use crate::id::Fd;
use crate::syscall::userptr::UserSlice;
use crate::syscall::{EBADF, EFAULT};

use crate::fs::file::OpenFlags;

/// `sys_channel_create` — create a channel pair and write `[fd_a, fd_b]` to user space.
///
/// `fds_ptr` points to a user buffer of at least `2 * size_of::<usize>()` bytes.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_channel_create(fds_ptr: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(fds_ptr, 2 * core::mem::size_of::<usize>()) else {
        return -EFAULT;
    };

    let (endpoint_a, endpoint_b) = crate::ipc::channel::channel();

    let (fd_a, fd_b) = crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        let a = fd_table.open(endpoint_a, OpenFlags::READ | OpenFlags::WRITE);
        let b = fd_table.open(endpoint_b, OpenFlags::READ | OpenFlags::WRITE);
        (a, b)
    });

    // SAFETY: UserSlice validated the pointer range is in user space.
    unsafe {
        let dst = user_slice.addr() as *mut usize;
        core::ptr::write(dst, fd_a.as_usize());
        core::ptr::write(dst.add(1), fd_b.as_usize());
    }

    0
}

/// `sys_channel_send` — send a message on a channel endpoint.
///
/// Enqueues one discrete message. If the send queue is full, triggers
/// TRAP_IO for async handling. Returns the message length on success.
#[expect(
    clippy::cast_possible_wrap,
    reason = "message lengths are small, wrap is impossible"
)]
pub(super) fn sys_channel_send(handle: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(handle as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_slice() };

    // Extract inode from fd table, then release the process lock before I/O.
    let inode = match crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return Err(-EBADF);
        };
        if !file.flags.contains(OpenFlags::WRITE) {
            return Err(-EBADF);
        }
        Ok(file.inode.clone())
    }) {
        Ok(inode) => inode,
        Err(e) => return e,
    };

    // Channel write enqueues a discrete message; offset is unused.
    match try_poll_immediate(inode.write(0, buf)) {
        Some(Ok(n)) => n as isize,
        Some(Err(e)) => {
            if matches!(e, crate::fs::FsError::BrokenPipe) {
                crate::proc::ProcessTable::with_current(|p| {
                    p.signals.post(crate::syscall::SIGPIPE);
                });
            }
            -e.to_errno()
        }
        None => {
            drop(inode);
            super::vfs::trap_io(fd, buf_ptr, buf_len, true)
        }
    }
}

/// `sys_channel_recv` — receive a message from a channel endpoint.
///
/// Dequeues one message. If the recv queue is empty, triggers TRAP_IO
/// for async handling. Returns the message length on success.
#[expect(
    clippy::cast_possible_wrap,
    reason = "message lengths are small, wrap is impossible"
)]
pub(super) fn sys_channel_recv(handle: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(handle as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_mut_slice() };

    // Extract inode from fd table, then release the process lock before I/O.
    let inode = match crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return Err(-EBADF);
        };
        if !file.flags.contains(OpenFlags::READ) {
            return Err(-EBADF);
        }
        Ok(file.inode.clone())
    }) {
        Ok(inode) => inode,
        Err(e) => return e,
    };

    // Channel read dequeues a discrete message; offset is unused.
    match try_poll_immediate(inode.read(0, buf)) {
        Some(Ok(n)) => n as isize,
        Some(Err(e)) => -e.to_errno(),
        None => {
            drop(inode);
            super::vfs::trap_io(fd, buf_ptr, buf_len, false)
        }
    }
}
