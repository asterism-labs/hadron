//! Channel syscall handlers: create, send, recv, send_fd, recv_fd.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use hadron_objects::channel::{Channel, ChannelError, ChannelMessage};
use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_objects::object::ObjectType;
use hadron_syscall::*;

use super::validate::{UserPtrMut, UserSlice};
use super::with_handle_table;

/// `SYS_CHANNEL_CREATE(fds_ptr)` — create a bidirectional channel pair.
///
/// Writes `[fd_a, fd_b]` to the user buffer at `fds_ptr`.
pub fn sys_channel_create(fds_ptr: usize) -> isize {
    let fds_out = match UserPtrMut::<[usize; 2]>::new(fds_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let (ch0, ch1) = Channel::create_pair();

    let result = with_handle_table(|table| {
        let hv0 = table.insert(HandleEntry::new(ch0, Rights::CHANNEL_DEFAULT))?;
        match table.insert(HandleEntry::new(ch1, Rights::CHANNEL_DEFAULT)) {
            Ok(hv1) => Ok((hv0, hv1)),
            Err(e) => {
                // Roll back the first insert.
                let _ = table.remove(hv0);
                Err(e)
            }
        }
    });

    match result {
        Ok((hv0, hv1)) => {
            // SAFETY: fds_ptr was validated by UserPtrMut::new.
            unsafe { fds_out.write([hv0.raw() as usize, hv1.raw() as usize]) };
            0
        }
        Err(_) => -EMFILE,
    }
}

/// `SYS_CHANNEL_SEND(fd, buf_ptr, len)` — send a message on a channel.
pub fn sys_channel_send(fd: usize, buf_ptr: usize, len: usize) -> isize {
    let slice = match UserSlice::new(buf_ptr, len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: User buffer was range-validated; pages are mapped (shared CR3).
    let data = unsafe { slice.read_to_vec() };

    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::WRITE) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let channel = match entry.object().as_any().downcast_ref::<Channel>() {
            Some(ch) => ch,
            None => return -EBADF,
        };

        let msg = ChannelMessage {
            data,
            handles: Vec::new(),
        };

        match channel.write(msg) {
            Ok(()) => {
                crate::process::wake_channel_recv_waiter();
                len as isize
            }
            Err(ChannelError::PeerClosed) => -EPIPE,
            Err(ChannelError::MessageTooLarge) => -EINVAL,
            Err(_) => -EIO,
        }
    })
}

/// `SYS_CHANNEL_RECV(fd, buf_ptr, len)` — receive a message from a channel.
///
/// If the channel is empty, blocks until a message is available by longjmping
/// back to the process task.
pub fn sys_channel_recv(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let out_slice = match UserSlice::new(buf_ptr, buf_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::READ) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let channel = match entry.object().as_any().downcast_ref::<Channel>() {
            Some(ch) => ch,
            None => return -EBADF,
        };

        match channel.read() {
            Ok(msg) => {
                let copy_len = msg.data.len().min(buf_len);
                // SAFETY: Output buffer was range-validated.
                unsafe { out_slice.write_from_slice(&msg.data[..copy_len]) };
                msg.data.len() as isize
            }
            Err(ChannelError::ShouldWait) => {
                // Block: set up blocking op and longjmp to process_task.
                crate::process::set_blocking_op(crate::process::BlockingOp::ChannelRecv {
                    fd,
                    buf_ptr,
                    buf_len,
                });
                let saved_rsp: u64;
                // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
                unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
                // SAFETY: saved_rsp is valid.
                unsafe {
                    crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
                }
            }
            Err(ChannelError::PeerClosed) => -EPIPE,
            Err(_) => -EIO,
        }
    })
}

/// `SYS_CHANNEL_SEND_FD(ch_fd, fd, buf_ptr, len)` — send with handle attachment.
pub fn sys_channel_send_fd(ch_fd: usize, fd: usize, buf_ptr: usize, len: usize) -> isize {
    let slice = match UserSlice::new(buf_ptr, len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: User buffer was range-validated.
    let data = unsafe { slice.read_to_vec() };

    let ch_hv = HandleValue::from_raw(ch_fd as u32);
    let fd_hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        // First pass: validate both handles and clone the object Arc.
        let channel_obj = {
            let ch_entry = match table.get_with_rights(ch_hv, Rights::WRITE) {
                Ok(e) => e,
                Err(_) => return -EBADF,
            };
            if ch_entry.object().object_type() != ObjectType::Channel {
                return -EBADF;
            }
            Arc::clone(ch_entry.object())
        };

        let transferred = {
            let fd_entry = match table.get_with_rights(fd_hv, Rights::TRANSFER) {
                Ok(e) => e,
                Err(_) => return -EBADF,
            };
            HandleEntry::new(Arc::clone(fd_entry.object()), fd_entry.rights())
        };

        // Remove the transferred handle from the sender's table.
        let _ = table.remove(fd_hv);

        // Now downcast and write — no outstanding borrows on table.
        let channel = channel_obj.as_any().downcast_ref::<Channel>().unwrap();

        let msg = ChannelMessage {
            data,
            handles: alloc::vec![transferred],
        };

        match channel.write(msg) {
            Ok(()) => {
                crate::process::wake_channel_recv_waiter();
                len as isize
            }
            Err(ChannelError::PeerClosed) => -EPIPE,
            Err(ChannelError::MessageTooLarge) => -EINVAL,
            Err(_) => -EIO,
        }
    })
}

/// `SYS_CHANNEL_RECV_FD(ch_fd, buf_ptr, len, fd_out_ptr)` — receive with handle.
pub fn sys_channel_recv_fd(
    ch_fd: usize,
    buf_ptr: usize,
    buf_len: usize,
    fd_out_ptr: usize,
) -> isize {
    let out_slice = match UserSlice::new(buf_ptr, buf_len) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let fd_out = match UserPtrMut::<usize>::new(fd_out_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let ch_hv = HandleValue::from_raw(ch_fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(ch_hv, Rights::READ) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let channel = match entry.object().as_any().downcast_ref::<Channel>() {
            Some(ch) => ch,
            None => return -EBADF,
        };

        match channel.read() {
            Ok(msg) => {
                let copy_len = msg.data.len().min(buf_len);
                // SAFETY: Output buffer was range-validated.
                unsafe { out_slice.write_from_slice(&msg.data[..copy_len]) };

                // Install transferred handles into the receiver's table.
                let received_fd = if let Some(handle) = msg.handles.into_iter().next() {
                    match table.insert(handle) {
                        Ok(hv) => hv.raw() as usize,
                        Err(_) => usize::MAX,
                    }
                } else {
                    usize::MAX // No handle attached
                };

                // SAFETY: fd_out_ptr was validated.
                unsafe { fd_out.write(received_fd) };
                msg.data.len() as isize
            }
            Err(ChannelError::ShouldWait) => {
                // Block: set up blocking op and longjmp to process_task.
                crate::process::set_blocking_op(crate::process::BlockingOp::ChannelRecvFd {
                    ch_fd,
                    buf_ptr,
                    buf_len,
                    fd_out_ptr,
                });
                let saved_rsp: u64;
                // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
                unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
                // SAFETY: saved_rsp is valid.
                unsafe {
                    crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
                }
            }
            Err(ChannelError::PeerClosed) => -EPIPE,
            Err(_) => -EIO,
        }
    })
}
