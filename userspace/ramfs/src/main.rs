//! In-memory filesystem server backed by a CPIO initrd archive.
//!
//! Receives the initrd data on a channel (handle 3) and serves file system
//! requests via the [`FsServer`] trait from `lepton-syslib`. Parses the CPIO
//! archive at startup to build an in-memory directory tree, then enters
//! the VFS server event loop.

#![no_std]
#![no_main]

extern crate alloc;
extern crate lepton_syslib;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use hadron_syscall::types::{DirEntryInfo, INODE_TYPE_DIR, INODE_TYPE_FILE, StatInfo};
use hadron_syscall::wrappers;
use hadron_syscall::{EBADF, EISDIR, ENOENT, ENOTDIR, EROFS};
use lepton_syslib::fs_server::{self, FsServer};

// ── Constants ────────────────────────────────────────────────────────

/// Handle number for the mount channel (FS server end).
const MOUNT_CHANNEL_HANDLE: u32 = 0;

/// Handle number for the initrd shared memory.
const INITRD_VMO_HANDLE: u32 = 3;

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

// ── FsServer trait implementation ────────────────────────────────────

impl FsServer for RamFs {
    fn open(&mut self, file_channel: u32, path: &str, _flags: u32) -> (i32, Vec<u8>) {
        let lookup_path = if path.is_empty() || path == "." {
            String::from("/")
        } else if path.starts_with('/') {
            String::from(path)
        } else {
            alloc::format!("/{path}")
        };

        if self.nodes.contains_key(&lookup_path) {
            self.open_files
                .insert(file_channel, OpenFile { path: lookup_path });
            (0, Vec::new())
        } else {
            (ENOENT as i32, Vec::new())
        }
    }

    fn read(&mut self, file_channel: u32, offset: u64, len: u64) -> (i32, Vec<u8>) {
        let path = match self.open_files.get(&file_channel) {
            Some(f) => &f.path,
            None => return (EBADF as i32, Vec::new()),
        };

        match self.nodes.get(path) {
            Some(FsNode::File { data, .. }) => {
                let off = offset as usize;
                if off >= data.len() {
                    (0, Vec::new())
                } else {
                    let end = core::cmp::min(off + len as usize, data.len());
                    (0, data[off..end].to_vec())
                }
            }
            Some(FsNode::Directory { .. }) => (EISDIR as i32, Vec::new()),
            None => (ENOENT as i32, Vec::new()),
        }
    }

    fn write(&mut self, _file_channel: u32, _offset: u64, _data: &[u8]) -> (i32, Vec<u8>) {
        (EROFS as i32, Vec::new())
    }

    fn stat(&mut self, file_channel: u32) -> (i32, Vec<u8>) {
        let path = match self.open_files.get(&file_channel) {
            Some(f) => &f.path,
            None => return (EBADF as i32, Vec::new()),
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
                (0, stat_to_bytes(&info))
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
                (0, stat_to_bytes(&info))
            }
            None => (ENOENT as i32, Vec::new()),
        }
    }

    fn readdir(&mut self, file_channel: u32, offset: u64, max_entries: u32) -> (i32, Vec<u8>) {
        let path = match self.open_files.get(&file_channel) {
            Some(f) => &f.path,
            None => return (EBADF as i32, Vec::new()),
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

                (0, reply_data)
            }
            Some(FsNode::File { .. }) => (ENOTDIR as i32, Vec::new()),
            None => (ENOENT as i32, Vec::new()),
        }
    }

    fn close(&mut self, file_channel: u32) {
        self.open_files.remove(&file_channel);
    }
}

/// Convert a `StatInfo` to bytes.
fn stat_to_bytes(info: &StatInfo) -> Vec<u8> {
    // SAFETY: StatInfo is repr(C), no padding.
    unsafe {
        core::slice::from_raw_parts(
            core::ptr::from_ref(info).cast::<u8>(),
            core::mem::size_of::<StatInfo>(),
        )
    }
    .to_vec()
}

// ── CPIO parsing ─────────────────────────────────────────────────────

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

    nodes.insert(
        String::from("/"),
        FsNode::Directory {
            children: Vec::new(),
            mode: 0o755,
        },
    );

    let mut offset = 0usize;
    while offset + CPIO_HEADER_SIZE <= cpio_data.len() {
        if &cpio_data[offset..offset + 6] != b"070701" {
            break;
        }

        let mode = parse_hex(cpio_data, offset + 14);
        let filesize = parse_hex(cpio_data, offset + 54) as usize;
        let namesize = parse_hex(cpio_data, offset + 94) as usize;

        let name_start = offset + CPIO_HEADER_SIZE;
        let name_end = name_start + namesize.saturating_sub(1);
        if name_end > cpio_data.len() {
            break;
        }

        let raw_name = core::str::from_utf8(&cpio_data[name_start..name_end]).unwrap_or("");

        if raw_name == "TRAILER!!!" {
            break;
        }

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

// ── Entry point ──────────────────────────────────────────────────────

/// Ramfs main: read initrd, build tree, serve requests.
#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    lepton_syslib::println!("ramfs: starting");

    let mut initrd_buf = vec![0u8; 512 * 1024];
    let n = wrappers::sys_channel_recv(
        INITRD_VMO_HANDLE as usize,
        initrd_buf.as_mut_ptr() as usize,
        initrd_buf.len(),
    );

    if n < 0 {
        lepton_syslib::println!("ramfs: failed to receive initrd data");
        return 1;
    }

    initrd_buf.truncate(n as usize);

    lepton_syslib::println!("ramfs: parsing initrd CPIO archive");
    let nodes = build_tree(&initrd_buf);
    lepton_syslib::println!("ramfs: tree built ({} nodes)", nodes.len());

    let mut server = RamFs {
        nodes,
        open_files: BTreeMap::new(),
    };

    lepton_syslib::println!("ramfs: ready");
    fs_server::run_fs_server(&mut server, MOUNT_CHANNEL_HANDLE);
}
