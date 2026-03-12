//! Stub device manager filesystem server.
//!
//! Serves `/dev` with two virtual entries: `null` and `console`.
//! - `null`: reads return EOF, writes are discarded.
//! - `console`: reads return EOF, writes go to debug log.
//!
//! Receives the mount channel on handle 0.

#![no_std]
#![no_main]

extern crate alloc;

mod heap;

#[global_allocator]
static HEAP: heap::UserHeap = heap::UserHeap::new();

use alloc::vec::Vec;

use hadron_syscall::wrappers;
use hadron_syscall::*;
use hadron_vfs_protocol::*;

// ── Constants ────────────────────────────────────────────────────────

/// Handle number for the mount channel.
const MOUNT_CHANNEL_HANDLE: u32 = 0;

/// Maximum message buffer size.
const MSG_BUF_SIZE: usize = 4096;

/// Maximum open files.
const MAX_OPEN_FILES: usize = 32;

// ── Device types ─────────────────────────────────────────────────────

/// Known device files.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DevFile {
    /// Root directory `/dev`.
    Root,
    /// `/dev/null` — reads return EOF, writes are discarded.
    Null,
    /// `/dev/console` — reads return EOF, writes go to debug log.
    Console,
}

/// Per-open-file state.
struct OpenFile {
    dev: DevFile,
}

/// Devmgr server state.
struct DevMgr {
    /// Channel handle → open file state.
    open_files: [Option<OpenFile>; MAX_OPEN_FILES],
    /// Channel handles for open files.
    file_channels: Vec<u32>,
}

// ── Helpers ──────────────────────────────────────────────────────────

fn debug_log(msg: &str) {
    wrappers::sys_debug_log(msg.as_ptr() as usize, msg.len());
}

fn send_reply(ch_fd: u32, status: i32, data: &[u8]) {
    let reply = VfsReply {
        status,
        data_len: data.len() as u32,
    };

    let reply_size = core::mem::size_of::<VfsReply>();
    let mut buf = Vec::with_capacity(reply_size + data.len());
    // SAFETY: VfsReply is repr(C).
    buf.extend_from_slice(unsafe {
        core::slice::from_raw_parts(core::ptr::from_ref(&reply).cast::<u8>(), reply_size)
    });
    buf.extend_from_slice(data);

    wrappers::sys_channel_send(ch_fd as usize, buf.as_ptr() as usize, buf.len());
}

/// Resolve a path to a device.
fn resolve_path(path: &str) -> Option<DevFile> {
    match path {
        "" | "/" | "." => Some(DevFile::Root),
        "null" | "/null" => Some(DevFile::Null),
        "console" | "/console" => Some(DevFile::Console),
        _ => None,
    }
}

// ── Request handlers ─────────────────────────────────────────────────

impl DevMgr {
    fn new() -> Self {
        Self {
            open_files: core::array::from_fn(|_| None),
            file_channels: Vec::new(),
        }
    }

    /// Find index for a channel handle.
    fn find_slot(&self, ch: u32) -> Option<usize> {
        self.file_channels.iter().position(|&c| c == ch)
    }

    fn handle_open(&mut self, file_ch: u32, path: &str, _flags: u32) -> bool {
        match resolve_path(path) {
            Some(dev) => {
                if self.file_channels.len() >= MAX_OPEN_FILES {
                    send_reply(file_ch, EMFILE as i32, &[]);
                    return false;
                }
                let idx = self.file_channels.len();
                self.file_channels.push(file_ch);
                self.open_files[idx] = Some(OpenFile { dev });
                send_reply(file_ch, 0, &[]);
                true
            }
            None => {
                send_reply(file_ch, ENOENT as i32, &[]);
                false
            }
        }
    }

    fn handle_read(&self, file_ch: u32, _offset: u64, _len: u64) {
        let idx = match self.find_slot(file_ch) {
            Some(i) => i,
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        match &self.open_files[idx] {
            Some(f) if f.dev == DevFile::Root => {
                send_reply(file_ch, EISDIR as i32, &[]);
            }
            Some(_) => {
                // null and console: read returns EOF (zero bytes).
                send_reply(file_ch, 0, &[]);
            }
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
            }
        }
    }

    fn handle_write(&self, file_ch: u32, _offset: u64, data: &[u8]) {
        let idx = match self.find_slot(file_ch) {
            Some(i) => i,
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        match &self.open_files[idx] {
            Some(f) if f.dev == DevFile::Console => {
                // Write to debug log.
                if let Ok(s) = core::str::from_utf8(data) {
                    debug_log(s);
                }
                let written = (data.len() as u64).to_le_bytes();
                send_reply(file_ch, 0, &written);
            }
            Some(f) if f.dev == DevFile::Null => {
                // Discard data.
                let written = (data.len() as u64).to_le_bytes();
                send_reply(file_ch, 0, &written);
            }
            Some(_) => {
                send_reply(file_ch, EISDIR as i32, &[]);
            }
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
            }
        }
    }

    fn handle_stat(&self, file_ch: u32) {
        let idx = match self.find_slot(file_ch) {
            Some(i) => i,
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        let info = match &self.open_files[idx] {
            Some(f) if f.dev == DevFile::Root => StatInfo {
                inode_type: INODE_TYPE_DIR,
                _pad: 0,
                size: 2,
                permissions: 0o755,
                nlinks: 1,
                dev: 0,
            },
            Some(f) if f.dev == DevFile::Console || f.dev == DevFile::Null => StatInfo {
                inode_type: INODE_TYPE_DEVICE,
                _pad: 0,
                size: 0,
                permissions: 0o666,
                nlinks: 1,
                dev: 0,
            },
            _ => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        let bytes = unsafe {
            core::slice::from_raw_parts(
                core::ptr::from_ref(&info).cast::<u8>(),
                core::mem::size_of::<StatInfo>(),
            )
        };
        send_reply(file_ch, 0, bytes);
    }

    fn handle_readdir(&self, file_ch: u32, offset: u64, _max_entries: u32) {
        let idx = match self.find_slot(file_ch) {
            Some(i) => i,
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        match &self.open_files[idx] {
            Some(f) if f.dev == DevFile::Root => {
                let entries = [("null", INODE_TYPE_DEVICE), ("console", INODE_TYPE_DEVICE)];
                let start = offset as usize;
                let mut reply_data = Vec::new();

                for &(name, inode_type) in entries.iter().skip(start) {
                    let mut entry = DirEntryInfo {
                        inode_type,
                        name_len: name.len() as u32,
                        name: [0u8; 256],
                    };
                    entry.name[..name.len()].copy_from_slice(name.as_bytes());

                    let entry_bytes = unsafe {
                        core::slice::from_raw_parts(
                            core::ptr::from_ref(&entry).cast::<u8>(),
                            core::mem::size_of::<DirEntryInfo>(),
                        )
                    };
                    reply_data.extend_from_slice(entry_bytes);
                }

                send_reply(file_ch, 0, &reply_data);
            }
            Some(_) => {
                send_reply(file_ch, ENOTDIR as i32, &[]);
            }
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
            }
        }
    }

    fn handle_close(&mut self, file_ch: u32) {
        if let Some(idx) = self.find_slot(file_ch) {
            self.open_files[idx] = None;
            self.file_channels.swap_remove(idx);
            // Fix up open_files after swap_remove.
            if idx < self.file_channels.len() {
                self.open_files[idx] = self.open_files[self.file_channels.len()].take();
            }
        }
    }
}

// ── Server loop ──────────────────────────────────────────────────────

fn handle_mount_msg(server: &mut DevMgr, mount_fd: u32) -> Option<u32> {
    let mut buf = [0u8; MSG_BUF_SIZE];
    let mut new_fd: usize = 0;

    let n = wrappers::sys_channel_recv_fd(
        mount_fd as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        &mut new_fd as *mut usize as usize,
    );

    if n < 0 || new_fd == 0 {
        return None;
    }

    let n = n as usize;
    let req_size = core::mem::size_of::<VfsRequest>();
    if n < req_size {
        return None;
    }

    let req: VfsRequest = unsafe { core::ptr::read_unaligned(buf.as_ptr().cast()) };
    if req.op != FS_OP_OPEN {
        send_reply(new_fd as u32, EINVAL as i32, &[]);
        return None;
    }

    let path_end = req_size + req.path_len as usize;
    if path_end > n {
        send_reply(new_fd as u32, EINVAL as i32, &[]);
        return None;
    }

    let path = core::str::from_utf8(&buf[req_size..path_end]).unwrap_or("");
    let file_ch = new_fd as u32;

    if server.handle_open(file_ch, path, req.flags) {
        Some(file_ch)
    } else {
        wrappers::sys_handle_close(file_ch as usize);
        None
    }
}

fn handle_file_msg(server: &mut DevMgr, file_fd: u32) -> bool {
    let mut buf = [0u8; MSG_BUF_SIZE];

    let n = wrappers::sys_channel_recv(file_fd as usize, buf.as_mut_ptr() as usize, buf.len());
    if n < 0 {
        return false;
    }

    let n = n as usize;
    let req_size = core::mem::size_of::<VfsRequest>();
    if n < req_size {
        send_reply(file_fd, EINVAL as i32, &[]);
        return true;
    }

    let req: VfsRequest = unsafe { core::ptr::read_unaligned(buf.as_ptr().cast()) };

    match req.op {
        FS_OP_READ => {
            let args_end = req_size + core::mem::size_of::<ReadArgs>();
            if args_end > n {
                send_reply(file_fd, EINVAL as i32, &[]);
            } else {
                let args: ReadArgs =
                    unsafe { core::ptr::read_unaligned(buf[req_size..].as_ptr().cast()) };
                server.handle_read(file_fd, args.offset, args.len);
            }
        }
        FS_OP_WRITE => {
            let args_end = req_size + core::mem::size_of::<WriteArgs>();
            if args_end > n {
                send_reply(file_fd, EINVAL as i32, &[]);
            } else {
                let args: WriteArgs =
                    unsafe { core::ptr::read_unaligned(buf[req_size..].as_ptr().cast()) };
                let data_start = args_end;
                let write_data = if data_start < n {
                    &buf[data_start..n]
                } else {
                    &[]
                };
                server.handle_write(file_fd, args.offset, write_data);
            }
        }
        FS_OP_STAT => {
            server.handle_stat(file_fd);
        }
        FS_OP_READDIR => {
            let args_end = req_size + core::mem::size_of::<ReaddirArgs>();
            if args_end > n {
                send_reply(file_fd, EINVAL as i32, &[]);
            } else {
                let args: ReaddirArgs =
                    unsafe { core::ptr::read_unaligned(buf[req_size..].as_ptr().cast()) };
                server.handle_readdir(file_fd, args.offset, args.max_entries);
            }
        }
        _ => {
            send_reply(file_fd, ENOSYS as i32, &[]);
        }
    }

    true
}

fn run_server(server: &mut DevMgr) -> ! {
    let mount_fd = MOUNT_CHANNEL_HANDLE;

    loop {
        let poll_count = 1 + server.file_channels.len();
        let mut poll_fds: Vec<PollFd> = Vec::with_capacity(poll_count);

        poll_fds.push(PollFd {
            fd: mount_fd,
            events: POLLIN,
            revents: 0,
        });

        for &ch in &server.file_channels {
            poll_fds.push(PollFd {
                fd: ch,
                events: POLLIN,
                revents: 0,
            });
        }

        let ready =
            wrappers::sys_event_wait_many(poll_fds.as_mut_ptr() as usize, poll_count, usize::MAX);

        if ready < 0 {
            continue;
        }

        // Check mount channel.
        if poll_fds[0].revents & POLLIN != 0 {
            if let Some(new_ch) = handle_mount_msg(server, mount_fd) {
                let _ = new_ch; // Already added to server.file_channels by handle_open.
            }
        }

        // Check per-file channels.
        let mut to_remove: Vec<usize> = Vec::new();
        // Snapshot the channel list length since we reference poll_fds.
        let ch_count = server.file_channels.len();
        for i in 0..ch_count {
            let ch = server.file_channels[i];
            if i + 1 < poll_fds.len() && poll_fds[i + 1].revents & (POLLIN | POLLHUP) != 0 {
                if poll_fds[i + 1].revents & POLLHUP != 0 {
                    to_remove.push(i);
                } else if !handle_file_msg(server, ch) {
                    to_remove.push(i);
                }
            }
        }

        for &i in to_remove.iter().rev() {
            let ch = server.file_channels[i];
            server.handle_close(ch);
            wrappers::sys_handle_close(ch as usize);
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────

#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "call {main}",
        "ud2",
        main = sym main,
    );
}

fn main() -> ! {
    debug_log("devmgr: starting\n");

    let mut server = DevMgr::new();

    debug_log("devmgr: ready\n");
    run_server(&mut server)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = info;
    debug_log("devmgr: PANIC!\n");
    wrappers::sys_task_exit(99);
}
