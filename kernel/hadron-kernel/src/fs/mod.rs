//! Virtual filesystem layer.
//!
//! Core VFS abstractions (traits, types, path utilities, devfs) live in the
//! `hadron-fs` crate for host testability. This module re-exports them and
//! adds kernel-specific code (DevConsole, block adapter, console input).

// Re-export everything from hadron-fs root.
pub use hadron_fs::{
    DirEntry, FileSystem, FsError, Inode, InodeType, Permissions, noop_waker, poll_immediate,
    try_poll_immediate,
};

// Re-export submodules that don't need kernel extension.
pub use hadron_fs::file;
pub use hadron_fs::path;

// Kernel-extended modules.
pub mod block_adapter;
pub mod console_input;
pub mod devfs;
pub mod vfs;
