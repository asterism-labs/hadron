//! VFS (vnode) syscall stubs.
//!
//! All vnode syscalls return `ENOSYS` until the VFS layer is implemented
//! in a later phase.

use hadron_syscall::ENOSYS;

/// `SYS_VNODE_OPEN` — stub.
pub fn sys_vnode_open(_path_ptr: usize, _path_len: usize, _flags: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_READ` — stub.
pub fn sys_vnode_read(_fd: usize, _buf_ptr: usize, _buf_len: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_WRITE` — stub.
pub fn sys_vnode_write(_fd: usize, _buf_ptr: usize, _buf_len: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_STAT` — stub.
pub fn sys_vnode_stat(_fd: usize, _buf_ptr: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_READDIR` — stub.
pub fn sys_vnode_readdir(_fd: usize, _buf_ptr: usize, _buf_len: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_UNLINK` — stub.
pub fn sys_vnode_unlink(_path_ptr: usize, _path_len: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_SEEK` — stub.
pub fn sys_vnode_seek(_fd: usize, _offset: usize, _whence: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_MKDIR` — stub.
pub fn sys_vnode_mkdir(_path_ptr: usize, _path_len: usize, _mode: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_RENAME` — stub.
pub fn sys_vnode_rename(
    _old_ptr: usize,
    _old_len: usize,
    _new_ptr: usize,
    _new_len: usize,
) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_SYMLINK` — stub.
pub fn sys_vnode_symlink(
    _target_ptr: usize,
    _target_len: usize,
    _link_ptr: usize,
    _link_len: usize,
) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_LINK` — stub.
pub fn sys_vnode_link(_old_ptr: usize, _old_len: usize, _new_ptr: usize, _new_len: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_READLINK` — stub.
pub fn sys_vnode_readlink(_path_ptr: usize, _path_len: usize, _buf_ptr: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_TRUNCATE` — stub.
pub fn sys_vnode_truncate(_fd: usize, _length: usize) -> isize {
    -ENOSYS
}

/// `SYS_VNODE_FSTATAT` — stub.
pub fn sys_vnode_fstatat(
    _dirfd: usize,
    _path_ptr: usize,
    _path_len: usize,
    _buf_ptr: usize,
) -> isize {
    -ENOSYS
}
