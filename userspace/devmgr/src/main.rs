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
extern crate lepton_syslib;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use hadron_syscall::types::{DirEntryInfo, INODE_TYPE_DEVICE, INODE_TYPE_DIR, StatInfo};
use hadron_syscall::wrappers;
use hadron_syscall::{EBADF, EISDIR, ENOENT, ENOTDIR};
use lepton_syslib::fs_server::{self, FsServer};

// ── Constants ────────────────────────────────────────────────────────

/// Handle number for the mount channel.
const MOUNT_CHANNEL_HANDLE: u32 = 0;

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

/// Devmgr server state.
struct DevMgr {
    /// Channel handle → device mapping.
    open_files: BTreeMap<u32, DevFile>,
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

// ── FsServer trait implementation ────────────────────────────────────

impl FsServer for DevMgr {
    fn open(&mut self, file_channel: u32, path: &str, _flags: u32) -> (i32, Vec<u8>) {
        match resolve_path(path) {
            Some(dev) => {
                self.open_files.insert(file_channel, dev);
                (0, Vec::new())
            }
            None => (ENOENT as i32, Vec::new()),
        }
    }

    fn read(&mut self, file_channel: u32, _offset: u64, _len: u64) -> (i32, Vec<u8>) {
        match self.open_files.get(&file_channel) {
            Some(&DevFile::Root) => (EISDIR as i32, Vec::new()),
            Some(_) => (0, Vec::new()), // null and console: EOF
            None => (EBADF as i32, Vec::new()),
        }
    }

    fn write(&mut self, file_channel: u32, _offset: u64, data: &[u8]) -> (i32, Vec<u8>) {
        match self.open_files.get(&file_channel) {
            Some(&DevFile::Console) => {
                if let Ok(s) = core::str::from_utf8(data) {
                    wrappers::sys_debug_log(s.as_ptr() as usize, s.len());
                }
                let written = (data.len() as u64).to_le_bytes();
                (0, written.to_vec())
            }
            Some(&DevFile::Null) => {
                let written = (data.len() as u64).to_le_bytes();
                (0, written.to_vec())
            }
            Some(&DevFile::Root) => (EISDIR as i32, Vec::new()),
            None => (EBADF as i32, Vec::new()),
        }
    }

    fn stat(&mut self, file_channel: u32) -> (i32, Vec<u8>) {
        let info = match self.open_files.get(&file_channel) {
            Some(&DevFile::Root) => StatInfo {
                inode_type: INODE_TYPE_DIR,
                _pad: 0,
                size: 2,
                permissions: 0o755,
                nlinks: 1,
                dev: 0,
            },
            Some(_) => StatInfo {
                inode_type: INODE_TYPE_DEVICE,
                _pad: 0,
                size: 0,
                permissions: 0o666,
                nlinks: 1,
                dev: 0,
            },
            None => return (EBADF as i32, Vec::new()),
        };

        // SAFETY: StatInfo is repr(C).
        let bytes = unsafe {
            core::slice::from_raw_parts(
                core::ptr::from_ref(&info).cast::<u8>(),
                core::mem::size_of::<StatInfo>(),
            )
        };
        (0, bytes.to_vec())
    }

    fn readdir(&mut self, file_channel: u32, offset: u64, _max_entries: u32) -> (i32, Vec<u8>) {
        match self.open_files.get(&file_channel) {
            Some(&DevFile::Root) => {
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
            Some(_) => (ENOTDIR as i32, Vec::new()),
            None => (EBADF as i32, Vec::new()),
        }
    }

    fn close(&mut self, file_channel: u32) {
        self.open_files.remove(&file_channel);
    }
}

// ── Entry point ──────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    lepton_syslib::println!("devmgr: starting");

    let mut server = DevMgr {
        open_files: BTreeMap::new(),
    };

    lepton_syslib::println!("devmgr: ready");
    fs_server::run_fs_server(&mut server, MOUNT_CHANNEL_HANDLE);
}
