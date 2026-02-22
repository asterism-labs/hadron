//! VFS tests — path resolution, device files, file operations.

extern crate alloc;

use alloc::vec;
use hadron_ktest::kernel_test;

use crate::fs::{FsError, InodeType, Permissions, poll_immediate};

// ── Before executor stage — sync tests ──────────────────────────────────

#[kernel_test(stage = "before_executor")]
fn test_vfs_root_exists() {
    let result = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/"));
    assert!(result.is_ok(), "VFS root should be resolvable");
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_root_is_directory() {
    let inode = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/")).expect("resolve /");
    assert_eq!(
        inode.inode_type(),
        InodeType::Directory,
        "root inode should be a directory"
    );
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_resolve_dev() {
    let result = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/dev"));
    assert!(result.is_ok(), "/dev should be resolvable (devfs mounted)");
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_dev_null_exists() {
    let inode = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/dev/null"))
        .expect("resolve /dev/null");
    assert_eq!(
        inode.inode_type(),
        InodeType::CharDevice,
        "/dev/null should be a CharDevice"
    );
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_dev_zero_exists() {
    let inode = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/dev/zero"))
        .expect("resolve /dev/zero");
    assert_eq!(
        inode.inode_type(),
        InodeType::CharDevice,
        "/dev/zero should be a CharDevice"
    );
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_dev_null_read() {
    let inode = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/dev/null"))
        .expect("resolve /dev/null");
    let mut buf = [0u8; 64];
    let n = poll_immediate(inode.read(0, &mut buf)).expect("read /dev/null");
    assert_eq!(n, 0, "reading /dev/null should return 0 bytes");
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_dev_zero_read() {
    let inode = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/dev/zero"))
        .expect("resolve /dev/zero");
    let mut buf = [0xFFu8; 64];
    let n = poll_immediate(inode.read(0, &mut buf)).expect("read /dev/zero");
    assert_eq!(n, 64, "/dev/zero should fill the buffer");
    assert!(buf.iter().all(|&b| b == 0), "/dev/zero should write zeros");
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_resolve_nonexistent() {
    let result = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/nonexistent"));
    match result {
        Err(FsError::NotFound) => {} // expected
        other => panic!("expected NotFound for /nonexistent, got {:?}", other.err()),
    }
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_create_file_in_root() {
    let root = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/")).expect("resolve /");
    let created = poll_immediate(root.create("ktest_file", InodeType::File, Permissions::all()))
        .expect("create file in root");
    assert_eq!(created.inode_type(), InodeType::File);

    // Lookup should now find it.
    let found = poll_immediate(root.lookup("ktest_file")).expect("lookup ktest_file");
    assert_eq!(found.inode_type(), InodeType::File);
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_write_and_read_file() {
    let root = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/")).expect("resolve /");
    let file = poll_immediate(root.create("ktest_rw", InodeType::File, Permissions::all()))
        .expect("create file");

    let data = b"hello from ktest";
    let written = poll_immediate(file.write(0, data)).expect("write");
    assert_eq!(written, data.len());

    let mut buf = vec![0u8; data.len()];
    let read = poll_immediate(file.read(0, &mut buf)).expect("read");
    assert_eq!(read, data.len());
    assert_eq!(&buf, data);
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_readdir_root() {
    let root = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/")).expect("resolve /");
    let entries = poll_immediate(root.readdir()).expect("readdir /");
    // The root should have at least some entries (mounted devfs creates /dev).
    assert!(!entries.is_empty(), "root readdir should return entries");
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_create_directory() {
    let root = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/")).expect("resolve /");
    let dir = poll_immediate(root.create("ktest_dir", InodeType::Directory, Permissions::all()))
        .expect("create directory");
    assert_eq!(dir.inode_type(), InodeType::Directory);
}

#[kernel_test(stage = "before_executor")]
fn test_vfs_unlink_file() {
    let root = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/")).expect("resolve /");
    let _file = poll_immediate(root.create("ktest_unlink", InodeType::File, Permissions::all()))
        .expect("create file");
    poll_immediate(root.unlink("ktest_unlink")).expect("unlink");

    let result = poll_immediate(root.lookup("ktest_unlink"));
    match result {
        Err(FsError::NotFound) => {} // expected
        other => panic!("expected NotFound after unlink, got {:?}", other.err()),
    }
}

// ── With executor stage — async test ────────────────────────────────────

#[kernel_test(stage = "with_executor")]
async fn test_vfs_async_file_roundtrip() {
    let root = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/")).expect("resolve /");

    let file = root
        .create("ktest_async_rw", InodeType::File, Permissions::all())
        .await
        .expect("async create");

    let data = b"async ktest roundtrip";
    let written = file.write(0, data).await.expect("async write");
    assert_eq!(written, data.len());

    let mut buf = alloc::vec![0u8; data.len()];
    let read = file.read(0, &mut buf).await.expect("async read");
    assert_eq!(read, data.len());
    assert_eq!(&buf, data);
}
