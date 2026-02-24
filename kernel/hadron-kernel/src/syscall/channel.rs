//! Channel IPC syscall handlers.
//!
//! Implements bidirectional message-oriented channels for inter-process
//! communication. Each channel has two endpoints; messages sent on one
//! endpoint are received on the other. Extended with fd-passing variants
//! and service accept.

use alloc::sync::Arc;

use crate::fs::file::OpenFlags;
use crate::fs::try_poll_immediate;
use crate::id::Fd;
use crate::ipc::channel::ChannelEndpoint;
use crate::ipc::service::ServiceListener;
use crate::syscall::userptr::UserSlice;
use crate::syscall::{EBADF, EFAULT, EINVAL};

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

/// `sys_channel_accept` — dequeue a pending connection from a `ServiceListener`.
///
/// Returns the new channel fd on success, or a negative errno on failure.
/// Non-blocking: returns `-EAGAIN` if no connections are pending.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_channel_accept(listener_fd: usize) -> isize {
    let fd = Fd::new(listener_fd as u32);

    // Look up the inode and downcast to ServiceListener.
    let inode = match crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        fd_table.get(fd).map(|f| f.inode.clone())
    }) {
        Some(inode) => inode,
        None => return -EBADF,
    };

    // Downcast to ServiceListener.
    let listener_ptr = Arc::as_ptr(&inode) as *const dyn crate::fs::Inode;
    // SAFETY: We check the pointer via dynamic downcast. The ServiceListener
    // type is the only type registered at /dev/compositor_listen.
    let listener: &ServiceListener = unsafe {
        let raw = listener_ptr as *const ServiceListener;
        &*raw
    };

    match listener.try_accept() {
        Some(accepted_inode) => {
            let new_fd = crate::proc::ProcessTable::with_current(|process| {
                let mut fd_table = process.fd_table.lock();
                fd_table.open(accepted_inode, OpenFlags::READ | OpenFlags::WRITE)
            });
            new_fd.as_u32() as isize
        }
        None => -EINVAL, // No pending connections (caller should poll first)
    }
}

/// `sys_channel_send_fd` — send a message with an attached file descriptor.
///
/// The attached fd's inode is cloned and sent along with the message data.
/// Returns the message length on success.
#[expect(
    clippy::cast_possible_wrap,
    reason = "message lengths are small, wrap is impossible"
)]
pub(super) fn sys_channel_send_fd(
    handle: usize,
    fd_to_send: usize,
    buf_ptr: usize,
    buf_len: usize,
) -> isize {
    let fd = Fd::new(handle as u32);
    let send_fd = Fd::new(fd_to_send as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_slice() };

    // Extract both the channel inode and the inode to attach.
    let (channel_inode, attached_inode) = match crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let ch = fd_table.get(fd).map(|f| f.inode.clone());
        let att = fd_table.get(send_fd).map(|f| f.inode.clone());
        match (ch, att) {
            (Some(c), Some(a)) => Ok((c, a)),
            _ => Err(-EBADF),
        }
    }) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    // Downcast to ChannelEndpoint for send_with_attachment.
    let endpoint_ptr = Arc::as_ptr(&channel_inode) as *const ChannelEndpoint;
    // SAFETY: The fd was opened as a channel endpoint. Caller is responsible
    // for passing a channel fd.
    let endpoint: &ChannelEndpoint = unsafe { &*endpoint_ptr };

    match endpoint.send_with_attachment(buf, Some(attached_inode)) {
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
            drop(channel_inode);
            super::vfs::trap_io(fd, buf_ptr, buf_len, true)
        }
    }
}

/// `sys_channel_recv_fd` — receive a message with an optional attached fd.
///
/// If the message has an attached inode, it is installed in the caller's fd
/// table and the fd number is written to `fd_out_ptr`. If no attachment,
/// `usize::MAX` is written.
#[expect(
    clippy::cast_possible_wrap,
    reason = "message lengths and fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_channel_recv_fd(
    handle: usize,
    buf_ptr: usize,
    buf_len: usize,
    fd_out_ptr: usize,
) -> isize {
    let fd = Fd::new(handle as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };
    let Ok(fd_out_slice) = UserSlice::new(fd_out_ptr, core::mem::size_of::<usize>()) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated the range.
    let buf = unsafe { user_slice.as_mut_slice() };

    let channel_inode = match crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        fd_table.get(fd).map(|f| f.inode.clone()).ok_or(-EBADF)
    }) {
        Ok(inode) => inode,
        Err(e) => return e,
    };

    // Downcast to ChannelEndpoint for recv_with_attachment.
    let endpoint_ptr = Arc::as_ptr(&channel_inode) as *const ChannelEndpoint;
    // SAFETY: The fd was opened as a channel endpoint.
    let endpoint: &ChannelEndpoint = unsafe { &*endpoint_ptr };

    match endpoint.recv_with_attachment(buf) {
        Some(Ok((data_len, attached))) => {
            let received_fd = match attached {
                Some(inode) => {
                    let new_fd = crate::proc::ProcessTable::with_current(|process| {
                        let mut fd_table = process.fd_table.lock();
                        fd_table.open(inode, OpenFlags::READ | OpenFlags::WRITE)
                    });
                    new_fd.as_usize()
                }
                None => usize::MAX,
            };
            // SAFETY: fd_out_slice validated the pointer range.
            unsafe {
                core::ptr::write(fd_out_slice.addr() as *mut usize, received_fd);
            }
            data_len as isize
        }
        Some(Err(e)) => -e.to_errno(),
        None => {
            drop(channel_inode);
            super::vfs::trap_io(fd, buf_ptr, buf_len, false)
        }
    }
}
