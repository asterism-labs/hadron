//! Filesystem server framework.
//!
//! Provides the [`FsServer`] trait and [`run_fs_server`] event loop for
//! implementing userspace filesystem servers on Hadron. Each server listens
//! on a mount channel for `FS_OP_OPEN` requests and multiplexes I/O on
//! per-file channels using `sys_event_wait_many`.

extern crate alloc;

use alloc::vec::Vec;

use hadron_syscall::wrappers::*;
use hadron_syscall::*;
use hadron_vfs_protocol::*;

/// Maximum number of simultaneous open files a server can track.
const MAX_OPEN_FILES: usize = 64;

/// Receive buffer size for channel messages.
const RECV_BUF_SIZE: usize = 4096;

/// Trait implemented by filesystem servers.
///
/// Each method receives the per-file channel handle and request arguments.
/// Methods return `(status, response_data)` where status 0 means success
/// and a positive value is an errno code. The event loop sends the reply.
pub trait FsServer {
    /// Handle an open request. `path` is relative to the mount point.
    /// Returns `(status, response_data)`.
    fn open(&mut self, file_channel: u32, path: &str, flags: u32) -> (i32, Vec<u8>);

    /// Handle a read request.
    /// Returns `(status, data_read)`.
    fn read(&mut self, file_channel: u32, offset: u64, len: u64) -> (i32, Vec<u8>);

    /// Handle a write request.
    /// Returns `(status, bytes_written_as_le_bytes)`.
    fn write(&mut self, file_channel: u32, offset: u64, data: &[u8]) -> (i32, Vec<u8>);

    /// Handle a stat request.
    /// Returns `(status, stat_info_bytes)`.
    fn stat(&mut self, file_channel: u32) -> (i32, Vec<u8>);

    /// Handle a readdir request.
    /// Returns `(status, dir_entry_bytes)`.
    fn readdir(&mut self, file_channel: u32, offset: u64, max_entries: u32) -> (i32, Vec<u8>);

    /// Handle a close notification (peer closed the per-file channel).
    fn close(&mut self, file_channel: u32);
}

/// Send a `VfsReply` on a per-file channel.
fn send_reply(ch_fd: u32, status: i32, data: &[u8]) {
    let reply = VfsReply {
        status,
        data_len: data.len() as u32,
    };

    let mut buf = Vec::with_capacity(core::mem::size_of::<VfsReply>() + data.len());
    // SAFETY: VfsReply is repr(C) with no padding.
    buf.extend_from_slice(unsafe { as_bytes(&reply) });
    buf.extend_from_slice(data);

    sys_channel_send(ch_fd as usize, buf.as_ptr() as usize, buf.len());
}

/// Handle a message received on the mount channel.
///
/// For `FS_OP_OPEN`, the kernel sends the server-end channel handle attached
/// to the message. We decode the request, call `FsServer::open`, send the
/// reply, and return the new channel handle (to be added to the poll set).
fn handle_mount_message(server: &mut dyn FsServer, mount_fd: u32) -> Option<u32> {
    let mut buf = [0u8; RECV_BUF_SIZE];
    let mut new_fd: usize = 0;

    let n = sys_channel_recv_fd(
        mount_fd as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        &mut new_fd as *mut usize as usize,
    );

    if n < 0 {
        return None;
    }

    let n = n as usize;
    if n < core::mem::size_of::<VfsRequest>() {
        // Malformed message — send error reply if we got a channel.
        if new_fd != 0 {
            send_reply(new_fd as u32, EINVAL as i32, &[]);
        }
        return None;
    }

    // SAFETY: We verified `n >= size_of::<VfsRequest>()`.
    let req: VfsRequest = unsafe { core::ptr::read_unaligned(buf.as_ptr().cast()) };

    if req.op != FS_OP_OPEN {
        // Only open requests arrive on the mount channel.
        if new_fd != 0 {
            send_reply(new_fd as u32, EINVAL as i32, &[]);
        }
        return None;
    }

    let header_size = core::mem::size_of::<VfsRequest>();
    let path_end = header_size + req.path_len as usize;
    if path_end > n {
        if new_fd != 0 {
            send_reply(new_fd as u32, EINVAL as i32, &[]);
        }
        return None;
    }

    let path = core::str::from_utf8(&buf[header_size..path_end]).unwrap_or("");
    let file_ch = new_fd as u32;

    let (status, data) = server.open(file_ch, path, req.flags);
    send_reply(file_ch, status, &data);

    if status == 0 {
        Some(file_ch)
    } else {
        // Open failed — close the server-end channel.
        sys_handle_close(file_ch as usize);
        None
    }
}

/// Handle a message received on a per-file channel.
///
/// Decodes the `VfsRequest` and dispatches to the appropriate `FsServer`
/// method. Returns `false` if the channel should be removed (peer closed).
fn handle_file_message(server: &mut dyn FsServer, file_fd: u32) -> bool {
    let mut buf = [0u8; RECV_BUF_SIZE];

    let n = sys_channel_recv(file_fd as usize, buf.as_mut_ptr() as usize, buf.len());

    if n < 0 {
        // Channel closed or error — remove from poll set.
        return false;
    }

    let n = n as usize;
    if n < core::mem::size_of::<VfsRequest>() {
        send_reply(file_fd, EINVAL as i32, &[]);
        return true;
    }

    // SAFETY: We verified size.
    let req: VfsRequest = unsafe { core::ptr::read_unaligned(buf.as_ptr().cast()) };
    let header_size = core::mem::size_of::<VfsRequest>();

    let (status, data) = match req.op {
        FS_OP_READ => {
            let args_end = header_size + core::mem::size_of::<ReadArgs>();
            if args_end > n {
                (EINVAL as i32, Vec::new())
            } else {
                let args: ReadArgs =
                    unsafe { core::ptr::read_unaligned(buf[header_size..].as_ptr().cast()) };
                server.read(file_fd, args.offset, args.len)
            }
        }
        FS_OP_WRITE => {
            let args_end = header_size + core::mem::size_of::<WriteArgs>();
            if args_end > n {
                (EINVAL as i32, Vec::new())
            } else {
                let args: WriteArgs =
                    unsafe { core::ptr::read_unaligned(buf[header_size..].as_ptr().cast()) };
                let data_start = args_end;
                let write_data = if data_start < n {
                    &buf[data_start..n]
                } else {
                    &[]
                };
                server.write(file_fd, args.offset, write_data)
            }
        }
        FS_OP_STAT => server.stat(file_fd),
        FS_OP_READDIR => {
            let args_end = header_size + core::mem::size_of::<ReaddirArgs>();
            if args_end > n {
                (EINVAL as i32, Vec::new())
            } else {
                let args: ReaddirArgs =
                    unsafe { core::ptr::read_unaligned(buf[header_size..].as_ptr().cast()) };
                server.readdir(file_fd, args.offset, args.max_entries)
            }
        }
        _ => (ENOSYS as i32, Vec::new()),
    };

    send_reply(file_fd, status, &data);
    true
}

/// Run the filesystem server event loop.
///
/// Listens on `mount_fd` for open requests, multiplexes per-file channels
/// via `sys_event_wait_many`, and dispatches to the `FsServer` trait methods.
///
/// This function never returns under normal operation.
pub fn run_fs_server(server: &mut dyn FsServer, mount_fd: u32) -> ! {
    let mut file_channels: Vec<u32> = Vec::new();

    loop {
        // Build poll set: mount channel + all open file channels.
        let poll_count = 1 + file_channels.len();
        let mut poll_fds: Vec<PollFd> = Vec::with_capacity(poll_count);

        // First entry is the mount channel.
        poll_fds.push(PollFd {
            fd: mount_fd,
            events: POLLIN,
            revents: 0,
        });

        // Add all open file channels.
        for &ch in &file_channels {
            poll_fds.push(PollFd {
                fd: ch,
                events: POLLIN,
                revents: 0,
            });
        }

        // Wait for any channel to become readable (block indefinitely).
        let ready = sys_event_wait_many(
            poll_fds.as_ptr() as usize,
            poll_count,
            usize::MAX, // infinite timeout
        );

        if ready < 0 {
            continue;
        }

        // Check mount channel for new open requests.
        if poll_fds[0].revents & POLLIN != 0 {
            // The kernel sets revents in-place over events for wait_many.
            // For simplicity, try to receive on the mount channel.
            if let Some(new_ch) = handle_mount_message(server, mount_fd) {
                if file_channels.len() < MAX_OPEN_FILES {
                    file_channels.push(new_ch);
                } else {
                    // Too many open files — close the new channel.
                    send_reply(new_ch, EMFILE as i32, &[]);
                    sys_handle_close(new_ch as usize);
                }
            }
        }

        // Check per-file channels for I/O requests or hangup.
        let mut to_remove: Vec<usize> = Vec::new();
        for (i, &ch) in file_channels.iter().enumerate() {
            let revents = poll_fds[i + 1].revents;
            if revents & POLLIN == 0 && revents & POLLHUP == 0 {
                continue;
            }

            if !handle_file_message(server, ch) {
                server.close(ch);
                sys_handle_close(ch as usize);
                to_remove.push(i);
            }
        }

        // Remove closed channels (reverse order to preserve indices).
        for &i in to_remove.iter().rev() {
            file_channels.swap_remove(i);
        }
    }
}
