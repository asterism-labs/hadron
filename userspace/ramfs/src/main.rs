//! In-memory filesystem server backed by a CPIO initrd archive.
//!
//! Receives the initrd data on a shared-memory VMO (handle 3) and serves
//! file system requests on the mount channel (handle 0). Parses the CPIO
//! archive at startup to build an in-memory directory tree, then enters
//! the VFS server event loop.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use hadron_syscall::wrappers;
use hadron_syscall::*;
use hadron_vfs_protocol::*;

// ── Constants ────────────────────────────────────────────────────────

/// Handle number for the mount channel (FS server end).
const MOUNT_CHANNEL_HANDLE: u32 = 0;

/// Handle number for the initrd shared memory.
const INITRD_VMO_HANDLE: u32 = 3;

/// Maximum message buffer size.
const MSG_BUF_SIZE: usize = 4096;

/// Maximum number of open file channels to track.
const MAX_OPEN_FILES: usize = 64;

// ── File system node types ───────────────────────────────────────────

/// A node in the in-memory filesystem tree.
enum FsNode {
    /// A regular file with its data.
    File { data: Vec<u8>, mode: u32 },
    /// A directory with child names.
    Directory { children: Vec<String>, mode: u32 },
}

/// Per-open-file state tracked by the server.
struct OpenFile {
    /// Path this file was opened with.
    path: String,
}

/// The ramfs server state.
struct RamFs {
    /// Path → node mapping.
    nodes: BTreeMap<String, FsNode>,
    /// Channel handle → open file state.
    open_files: BTreeMap<u32, OpenFile>,
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Write a string to the kernel debug log.
fn debug_log(msg: &str) {
    wrappers::sys_debug_log(msg.as_ptr() as usize, msg.len());
}

/// Send a VFS reply on a channel.
fn send_reply(ch_fd: u32, status: i32, data: &[u8]) {
    let reply = VfsReply {
        status,
        data_len: data.len() as u32,
    };

    let reply_size = core::mem::size_of::<VfsReply>();
    let mut buf = Vec::with_capacity(reply_size + data.len());
    // SAFETY: VfsReply is repr(C), no padding.
    buf.extend_from_slice(unsafe {
        core::slice::from_raw_parts(core::ptr::from_ref(&reply).cast::<u8>(), reply_size)
    });
    buf.extend_from_slice(data);

    wrappers::sys_channel_send(ch_fd as usize, buf.as_ptr() as usize, buf.len());
}

// ── CPIO parsing into FsNode tree ────────────────────────────────────

/// CPIO newc header size.
const CPIO_HEADER_SIZE: usize = 110;

/// Parse an 8-character hex field from a CPIO header.
fn parse_hex(data: &[u8], offset: usize) -> u32 {
    let mut val = 0u32;
    for &b in &data[offset..offset + 8] {
        val = val << 4
            | match b {
                b'0'..=b'9' => u32::from(b - b'0'),
                b'a'..=b'f' => u32::from(b - b'a' + 10),
                b'A'..=b'F' => u32::from(b - b'A' + 10),
                _ => 0,
            };
    }
    val
}

/// Align a value up to a 4-byte boundary.
const fn align4(v: usize) -> usize {
    (v + 3) & !3
}

/// Build the in-memory filesystem tree from CPIO data.
fn build_tree(cpio_data: &[u8]) -> BTreeMap<String, FsNode> {
    let mut nodes: BTreeMap<String, FsNode> = BTreeMap::new();

    // Ensure root directory exists.
    nodes.insert(
        String::from("/"),
        FsNode::Directory {
            children: Vec::new(),
            mode: 0o755,
        },
    );

    let mut offset = 0usize;
    while offset + CPIO_HEADER_SIZE <= cpio_data.len() {
        // Validate magic.
        if &cpio_data[offset..offset + 6] != b"070701" {
            break;
        }

        let mode = parse_hex(cpio_data, offset + 14);
        let filesize = parse_hex(cpio_data, offset + 54) as usize;
        let namesize = parse_hex(cpio_data, offset + 94) as usize;

        let name_start = offset + CPIO_HEADER_SIZE;
        let name_end = name_start + namesize.saturating_sub(1); // exclude NUL
        if name_end > cpio_data.len() {
            break;
        }

        let raw_name = core::str::from_utf8(&cpio_data[name_start..name_end]).unwrap_or("");

        // Check for trailer.
        if raw_name == "TRAILER!!!" {
            break;
        }

        // Strip leading "./" prefix.
        let clean_name = raw_name
            .strip_prefix("./")
            .unwrap_or(raw_name)
            .trim_start_matches('/');

        let data_offset = align4(name_start + namesize);
        let data_end = data_offset + filesize;
        if data_end > cpio_data.len() {
            break;
        }

        if !clean_name.is_empty() {
            let abs_path = alloc::format!("/{clean_name}");

            let is_dir = mode & 0o170_000 == 0o040_000;

            if is_dir {
                nodes
                    .entry(abs_path.clone())
                    .or_insert_with(|| FsNode::Directory {
                        children: Vec::new(),
                        mode: mode & 0o7777,
                    });
            } else {
                let file_data = cpio_data[data_offset..data_end].to_vec();
                nodes.insert(
                    abs_path.clone(),
                    FsNode::File {
                        data: file_data,
                        mode: mode & 0o7777,
                    },
                );
            }

            // Register in parent directory.
            let parent = if let Some(slash_pos) = abs_path.rfind('/') {
                if slash_pos == 0 {
                    String::from("/")
                } else {
                    String::from(&abs_path[..slash_pos])
                }
            } else {
                String::from("/")
            };

            let child_name = String::from(abs_path.rsplit('/').next().unwrap_or(&abs_path));

            // Ensure parent directory exists.
            let parent_node = nodes.entry(parent).or_insert_with(|| FsNode::Directory {
                children: Vec::new(),
                mode: 0o755,
            });

            if let FsNode::Directory { children, .. } = parent_node {
                if !children.iter().any(|c| *c == child_name) {
                    children.push(child_name);
                }
            }
        }

        offset = align4(data_end);
    }

    nodes
}

// ── Server event loop ────────────────────────────────────────────────

impl RamFs {
    fn handle_open(&mut self, file_ch: u32, path: &str, _flags: u32) {
        // Normalize path — treat empty as root.
        let lookup_path = if path.is_empty() || path == "." {
            String::from("/")
        } else if path.starts_with('/') {
            String::from(path)
        } else {
            alloc::format!("/{path}")
        };

        if self.nodes.contains_key(&lookup_path) {
            self.open_files
                .insert(file_ch, OpenFile { path: lookup_path });
            send_reply(file_ch, 0, &[]);
        } else {
            send_reply(file_ch, ENOENT as i32, &[]);
        }
    }

    fn handle_read(&self, file_ch: u32, offset: u64, len: u64) {
        let path = match self.open_files.get(&file_ch) {
            Some(f) => &f.path,
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        match self.nodes.get(path) {
            Some(FsNode::File { data, .. }) => {
                let off = offset as usize;
                if off >= data.len() {
                    // EOF — return zero bytes.
                    send_reply(file_ch, 0, &[]);
                } else {
                    let end = core::cmp::min(off + len as usize, data.len());
                    send_reply(file_ch, 0, &data[off..end]);
                }
            }
            Some(FsNode::Directory { .. }) => {
                send_reply(file_ch, EISDIR as i32, &[]);
            }
            None => {
                send_reply(file_ch, ENOENT as i32, &[]);
            }
        }
    }

    fn handle_write(&self, file_ch: u32, _offset: u64, _data: &[u8]) {
        // Ramfs is read-only for Phase 5.
        send_reply(file_ch, EROFS as i32, &[]);
    }

    fn handle_stat(&self, file_ch: u32) {
        let path = match self.open_files.get(&file_ch) {
            Some(f) => &f.path,
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        match self.nodes.get(path) {
            Some(FsNode::File { data, mode }) => {
                let info = StatInfo {
                    inode_type: INODE_TYPE_FILE,
                    _pad: 0,
                    size: data.len() as u64,
                    permissions: *mode,
                    nlinks: 1,
                    dev: 0,
                };
                // SAFETY: StatInfo is repr(C), no padding.
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        core::ptr::from_ref(&info).cast::<u8>(),
                        core::mem::size_of::<StatInfo>(),
                    )
                };
                send_reply(file_ch, 0, bytes);
            }
            Some(FsNode::Directory { children, mode }) => {
                let info = StatInfo {
                    inode_type: INODE_TYPE_DIR,
                    _pad: 0,
                    size: children.len() as u64,
                    permissions: *mode,
                    nlinks: 1,
                    dev: 0,
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        core::ptr::from_ref(&info).cast::<u8>(),
                        core::mem::size_of::<StatInfo>(),
                    )
                };
                send_reply(file_ch, 0, bytes);
            }
            None => {
                send_reply(file_ch, ENOENT as i32, &[]);
            }
        }
    }

    fn handle_readdir(&self, file_ch: u32, offset: u64, max_entries: u32) {
        let path = match self.open_files.get(&file_ch) {
            Some(f) => &f.path,
            None => {
                send_reply(file_ch, EBADF as i32, &[]);
                return;
            }
        };

        match self.nodes.get(path) {
            Some(FsNode::Directory { children, .. }) => {
                let start = offset as usize;
                let count =
                    core::cmp::min(max_entries as usize, children.len().saturating_sub(start));
                let mut reply_data = Vec::new();

                for i in start..start + count {
                    let child_name = &children[i];
                    let child_path = if path == "/" {
                        alloc::format!("/{child_name}")
                    } else {
                        alloc::format!("{path}/{child_name}")
                    };

                    let inode_type = match self.nodes.get(&child_path) {
                        Some(FsNode::File { .. }) => INODE_TYPE_FILE,
                        Some(FsNode::Directory { .. }) => INODE_TYPE_DIR,
                        None => INODE_TYPE_FILE,
                    };

                    let mut entry = DirEntryInfo {
                        inode_type,
                        name_len: child_name.len() as u32,
                        name: [0u8; 256],
                    };
                    let name_bytes = child_name.as_bytes();
                    let copy_len = core::cmp::min(name_bytes.len(), 256);
                    entry.name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

                    // SAFETY: DirEntryInfo is repr(C).
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
            Some(FsNode::File { .. }) => {
                send_reply(file_ch, ENOTDIR as i32, &[]);
            }
            None => {
                send_reply(file_ch, ENOENT as i32, &[]);
            }
        }
    }

    fn handle_close(&mut self, file_ch: u32) {
        self.open_files.remove(&file_ch);
    }
}

// ── Server main loop ─────────────────────────────────────────────────

/// Handle a message from the mount channel (new open request).
/// Returns the new per-file channel handle if successful.
fn handle_mount_msg(server: &mut RamFs, mount_fd: u32) -> Option<u32> {
    let mut buf = [0u8; MSG_BUF_SIZE];
    let mut new_fd: usize = 0;

    let n = wrappers::sys_channel_recv_fd(
        mount_fd as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
        &mut new_fd as *mut usize as usize,
    );

    if n < 0 {
        return None;
    }

    let n = n as usize;
    let req_size = core::mem::size_of::<VfsRequest>();
    if n < req_size || new_fd == 0 {
        return None;
    }

    // SAFETY: We validated the size.
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

    server.handle_open(file_ch, path, req.flags);

    // Check if the open succeeded (file channel is now tracked).
    if server.open_files.contains_key(&file_ch) {
        Some(file_ch)
    } else {
        wrappers::sys_handle_close(file_ch as usize);
        None
    }
}

/// Handle a message on a per-file channel.
/// Returns false if the channel should be removed (peer closed).
fn handle_file_msg(server: &mut RamFs, file_fd: u32) -> bool {
    let mut buf = [0u8; MSG_BUF_SIZE];

    let n = wrappers::sys_channel_recv(file_fd as usize, buf.as_mut_ptr() as usize, buf.len());

    if n < 0 {
        // Channel closed or error.
        return false;
    }

    let n = n as usize;
    let req_size = core::mem::size_of::<VfsRequest>();
    if n < req_size {
        send_reply(file_fd, EINVAL as i32, &[]);
        return true;
    }

    // SAFETY: We validated size.
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

/// Run the ramfs server loop.
fn run_server(server: &mut RamFs) -> ! {
    let mount_fd = MOUNT_CHANNEL_HANDLE;
    let mut file_channels: Vec<u32> = Vec::new();

    loop {
        // Build poll set.
        let poll_count = 1 + file_channels.len();
        let mut poll_fds: Vec<PollFd> = Vec::with_capacity(poll_count);

        poll_fds.push(PollFd {
            fd: mount_fd,
            events: POLLIN,
            revents: 0,
        });

        for &ch in &file_channels {
            poll_fds.push(PollFd {
                fd: ch,
                events: POLLIN,
                revents: 0,
            });
        }

        // Block until something is ready.
        let ready =
            wrappers::sys_event_wait_many(poll_fds.as_mut_ptr() as usize, poll_count, usize::MAX);

        if ready < 0 {
            continue;
        }

        // Check mount channel.
        if poll_fds[0].revents & POLLIN != 0 {
            if let Some(new_ch) = handle_mount_msg(server, mount_fd) {
                if file_channels.len() < MAX_OPEN_FILES {
                    file_channels.push(new_ch);
                } else {
                    send_reply(new_ch, EMFILE as i32, &[]);
                    wrappers::sys_handle_close(new_ch as usize);
                }
            }
        }

        // Check per-file channels.
        let mut to_remove: Vec<usize> = Vec::new();
        for (i, &ch) in file_channels.iter().enumerate() {
            // poll_fds[i+1] corresponds to file_channels[i].
            if i + 1 < poll_fds.len() && poll_fds[i + 1].revents & (POLLIN | POLLHUP) != 0 {
                if poll_fds[i + 1].revents & POLLHUP != 0 {
                    // Peer closed.
                    server.handle_close(ch);
                    wrappers::sys_handle_close(ch as usize);
                    to_remove.push(i);
                } else if !handle_file_msg(server, ch) {
                    server.handle_close(ch);
                    wrappers::sys_handle_close(ch as usize);
                    to_remove.push(i);
                }
            }
        }

        for &i in to_remove.iter().rev() {
            file_channels.swap_remove(i);
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────

/// Raw entry point.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "call {main}",
        "ud2",
        main = sym main,
    );
}

/// Ramfs main: read initrd, build tree, serve requests.
fn main() -> ! {
    debug_log("ramfs: starting\n");

    // For Phase 5, the initrd data is passed as a shared memory region.
    // Map the VMO into our address space to read the CPIO archive.
    //
    // The kernel passes the initrd size in the first 8 bytes of a message
    // on handle 3, followed by the initrd data in the shared memory mapping.
    //
    // For now, receive the initrd data directly from the channel as a message.
    let mut initrd_buf = vec![0u8; 512 * 1024]; // 512 KiB max
    let n = wrappers::sys_channel_recv(
        INITRD_VMO_HANDLE as usize,
        initrd_buf.as_mut_ptr() as usize,
        initrd_buf.len(),
    );

    if n < 0 {
        debug_log("ramfs: failed to receive initrd data\n");
        wrappers::sys_task_exit(1);
    }

    let initrd_len = n as usize;
    initrd_buf.truncate(initrd_len);

    debug_log("ramfs: parsing initrd CPIO archive\n");
    let nodes = build_tree(&initrd_buf);

    let node_count = nodes.len();
    debug_log("ramfs: tree built (");
    // Print node count as a simple digit string.
    let mut count_buf = [0u8; 10];
    let count_str = format_usize(node_count, &mut count_buf);
    debug_log(count_str);
    debug_log(" nodes)\n");

    let mut server = RamFs {
        nodes,
        open_files: BTreeMap::new(),
    };

    debug_log("ramfs: ready\n");
    run_server(&mut server)
}

/// Format a usize as a decimal string into a fixed buffer.
fn format_usize(mut val: usize, buf: &mut [u8; 10]) -> &str {
    if val == 0 {
        return "0";
    }
    let mut i = buf.len();
    while val > 0 && i > 0 {
        i -= 1;
        buf[i] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    core::str::from_utf8(&buf[i..]).unwrap_or("?")
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = info;
    debug_log("ramfs: PANIC!\n");
    wrappers::sys_task_exit(99);
}
