//! VFS (vnode) syscall handlers.
//!
//! Implements `vnode_open`, `read`, `write`, `stat`, `readdir`, `seek` as
//! blocking syscalls that route through the VfsRouter to FS servers. Also
//! implements `vfs_mount` and `vfs_unmount`.
//!
//! Remaining vnode ops (unlink, mkdir, rename, etc.) return `ENOSYS`.

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec::Vec;

use hadron_objects::channel::{Channel, ChannelMessage};
use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_objects::object::KernelObject;
use hadron_objects::vnode::Vnode;
use hadron_syscall::*;
use hadron_vfs_protocol::*;

use super::validate::UserSlice;
use crate::process::{self, BlockingOp};
use crate::vfs;

/// `SYS_VNODE_OPEN(path_ptr, path_len, flags)` — open a file or directory.
///
/// Creates a channel pair, sends an open request (with the server end) on the
/// mount channel, blocks for the reply, then wraps the kernel end in a Vnode.
pub fn sys_vnode_open(path_ptr: usize, path_len: usize, flags: usize) -> isize {
    let path_slice = match UserSlice::new(path_ptr, path_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: User buffer was range-validated.
    let path_bytes = unsafe { path_slice.read_to_vec() };
    let raw_path = match core::str::from_utf8(&path_bytes) {
        Ok(p) => p,
        Err(_) => return -EINVAL,
    };

    // Prepend CWD if relative path.
    let abs_path = if raw_path.starts_with('/') {
        alloc::string::String::from(raw_path)
    } else {
        let cwd = process::with_current_process(|p| p.cwd()).unwrap_or_default();
        if cwd == "/" {
            alloc::format!("/{raw_path}")
        } else {
            alloc::format!("{cwd}/{raw_path}")
        }
    };

    let normalized = vfs::normalize_path(&abs_path);

    // Snapshot namespace and resolve path to mount channel.
    let namespace = match process::with_current_process(|p| p.namespace_snapshot()) {
        Some(ns) => ns,
        None => return -ESRCH,
    };

    let (mount_channel, relative) =
        match vfs::with(|router| router.resolve(&normalized, &namespace)) {
            Ok(r) => r,
            Err(vfs::VfsError::NotFound) => return -ENOENT,
            Err(vfs::VfsError::AccessDenied) => return -EACCES,
            Err(vfs::VfsError::AlreadyMounted) => return -EEXIST,
        };

    // Create per-file channel pair.
    let (kern_end, srv_end) = Channel::create_pair();

    // Build open request message.
    let open_flags = flags as u32;
    let req = VfsRequest {
        op: FS_OP_OPEN,
        flags: open_flags,
        path_len: relative.len() as u32,
    };

    let mut msg_data = Vec::new();
    // SAFETY: VfsRequest is repr(C) with no padding.
    msg_data.extend_from_slice(unsafe { as_bytes(&req) });
    msg_data.extend_from_slice(relative.as_bytes());

    // Send the server end as a handle attachment on the mount channel.
    let srv_entry = HandleEntry::new(srv_end as Arc<dyn KernelObject>, Rights::CHANNEL_DEFAULT);
    let msg = ChannelMessage {
        data: msg_data,
        handles: alloc::vec![srv_entry],
    };

    if let Err(_) = mount_channel.write(msg) {
        return -EIO;
    }

    // Block waiting for the FS server reply on the kernel end.
    let vnode = Vnode::new(kern_end, open_flags);
    let vnode_handle = match super::with_handle_table(|ht| {
        ht.insert(HandleEntry::new(
            vnode as Arc<dyn KernelObject>,
            Rights::VNODE_DEFAULT,
        ))
    }) {
        Ok(hv) => hv,
        Err(_) => return -EMFILE,
    };

    // Store blocking op to wait for the open reply.
    process::set_blocking_op(BlockingOp::VnodeOp {
        vnode_handle,
        buf_ptr: 0,
        buf_len: 0,
        syscall_nr: SYS_VNODE_OPEN as u32,
    });

    // Longjmp back to process task.
    let saved_rsp: u64;
    // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
    // SAFETY: saved_rsp is valid.
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// `SYS_VNODE_READ(fd, buf_ptr, buf_len)` — read from an open file.
pub fn sys_vnode_read(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    // Validate user buffer.
    if buf_len > 0 {
        if let Err(e) = UserSlice::new(buf_ptr, buf_len) {
            return e;
        }
    }

    // Verify handle exists and is a Vnode with READ right.
    let result: Result<_, hadron_objects::handle::HandleError> = super::with_handle_table(|ht| {
        let entry = ht.get_with_rights(hv, Rights::READ)?;
        let vnode = entry
            .object()
            .as_any()
            .downcast_ref::<Vnode>()
            .ok_or(hadron_objects::handle::HandleError::NotFound)?;
        Ok((Arc::clone(vnode.channel()), vnode.seek_offset()))
    });
    let (vnode_channel, seek_offset) = match result {
        Ok(r) => r,
        Err(_) => return -EBADF,
    };

    // Build read request.
    let req = VfsRequest {
        op: FS_OP_READ,
        flags: 0,
        path_len: 0,
    };
    let args = ReadArgs {
        offset: seek_offset,
        len: buf_len as u64,
    };

    let mut msg_data = Vec::new();
    // SAFETY: Both types are repr(C).
    msg_data.extend_from_slice(unsafe { as_bytes(&req) });
    msg_data.extend_from_slice(unsafe { as_bytes(&args) });

    let msg = ChannelMessage {
        data: msg_data,
        handles: Vec::new(),
    };

    if let Err(_) = vnode_channel.write(msg) {
        return -EIO;
    }

    // Block for reply.
    process::set_blocking_op(BlockingOp::VnodeOp {
        vnode_handle: hv,
        buf_ptr,
        buf_len,
        syscall_nr: SYS_VNODE_READ as u32,
    });

    let saved_rsp: u64;
    // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
    // SAFETY: saved_rsp is valid.
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// `SYS_VNODE_WRITE(fd, buf_ptr, buf_len)` — write to an open file.
pub fn sys_vnode_write(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    let write_data = if buf_len > 0 {
        let slice = match UserSlice::new(buf_ptr, buf_len) {
            Ok(s) => s,
            Err(e) => return e,
        };
        // SAFETY: User buffer was range-validated.
        unsafe { slice.read_to_vec() }
    } else {
        Vec::new()
    };

    let result: Result<_, hadron_objects::handle::HandleError> = super::with_handle_table(|ht| {
        let entry = ht.get_with_rights(hv, Rights::WRITE)?;
        let vnode = entry
            .object()
            .as_any()
            .downcast_ref::<Vnode>()
            .ok_or(hadron_objects::handle::HandleError::NotFound)?;
        Ok((Arc::clone(vnode.channel()), vnode.seek_offset()))
    });
    let (vnode_channel, seek_offset) = match result {
        Ok(r) => r,
        Err(_) => return -EBADF,
    };

    let req = VfsRequest {
        op: FS_OP_WRITE,
        flags: 0,
        path_len: 0,
    };
    let args = WriteArgs {
        offset: seek_offset,
    };

    let mut msg_data = Vec::new();
    // SAFETY: Both types are repr(C).
    msg_data.extend_from_slice(unsafe { as_bytes(&req) });
    msg_data.extend_from_slice(unsafe { as_bytes(&args) });
    msg_data.extend_from_slice(&write_data);

    let msg = ChannelMessage {
        data: msg_data,
        handles: Vec::new(),
    };

    if let Err(_) = vnode_channel.write(msg) {
        return -EIO;
    }

    process::set_blocking_op(BlockingOp::VnodeOp {
        vnode_handle: hv,
        buf_ptr: 0,
        buf_len: 0,
        syscall_nr: SYS_VNODE_WRITE as u32,
    });

    let saved_rsp: u64;
    // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
    // SAFETY: saved_rsp is valid.
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// `SYS_VNODE_STAT(fd, buf_ptr)` — get file metadata.
pub fn sys_vnode_stat(fd: usize, buf_ptr: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    if let Err(e) = UserSlice::new(buf_ptr, core::mem::size_of::<StatInfo>()) {
        return e;
    }

    let result: Result<_, hadron_objects::handle::HandleError> = super::with_handle_table(|ht| {
        let entry = ht.get_with_rights(hv, Rights::READ)?;
        let vnode = entry
            .object()
            .as_any()
            .downcast_ref::<Vnode>()
            .ok_or(hadron_objects::handle::HandleError::NotFound)?;
        Ok(Arc::clone(vnode.channel()))
    });
    let vnode_channel = match result {
        Ok(ch) => ch,
        Err(_) => return -EBADF,
    };

    let req = VfsRequest {
        op: FS_OP_STAT,
        flags: 0,
        path_len: 0,
    };

    let mut msg_data = Vec::new();
    // SAFETY: VfsRequest is repr(C).
    msg_data.extend_from_slice(unsafe { as_bytes(&req) });

    let msg = ChannelMessage {
        data: msg_data,
        handles: Vec::new(),
    };

    if let Err(_) = vnode_channel.write(msg) {
        return -EIO;
    }

    process::set_blocking_op(BlockingOp::VnodeOp {
        vnode_handle: hv,
        buf_ptr,
        buf_len: core::mem::size_of::<StatInfo>(),
        syscall_nr: SYS_VNODE_STAT as u32,
    });

    let saved_rsp: u64;
    // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
    // SAFETY: saved_rsp is valid.
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// `SYS_VNODE_READDIR(fd, buf_ptr, buf_len)` — read directory entries.
pub fn sys_vnode_readdir(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    if buf_len > 0 {
        if let Err(e) = UserSlice::new(buf_ptr, buf_len) {
            return e;
        }
    }

    let result: Result<_, hadron_objects::handle::HandleError> = super::with_handle_table(|ht| {
        let entry = ht.get_with_rights(hv, Rights::READ)?;
        let vnode = entry
            .object()
            .as_any()
            .downcast_ref::<Vnode>()
            .ok_or(hadron_objects::handle::HandleError::NotFound)?;
        Ok((Arc::clone(vnode.channel()), vnode.seek_offset()))
    });
    let (vnode_channel, seek_offset) = match result {
        Ok(r) => r,
        Err(_) => return -EBADF,
    };

    let entry_size = core::mem::size_of::<DirEntryInfo>();
    let max_entries = if entry_size > 0 {
        buf_len / entry_size
    } else {
        0
    };

    let req = VfsRequest {
        op: FS_OP_READDIR,
        flags: 0,
        path_len: 0,
    };
    let args = ReaddirArgs {
        offset: seek_offset,
        max_entries: max_entries as u32,
        _pad: 0,
    };

    let mut msg_data = Vec::new();
    // SAFETY: Both types are repr(C).
    msg_data.extend_from_slice(unsafe { as_bytes(&req) });
    msg_data.extend_from_slice(unsafe { as_bytes(&args) });

    let msg = ChannelMessage {
        data: msg_data,
        handles: Vec::new(),
    };

    if let Err(_) = vnode_channel.write(msg) {
        return -EIO;
    }

    process::set_blocking_op(BlockingOp::VnodeOp {
        vnode_handle: hv,
        buf_ptr,
        buf_len,
        syscall_nr: SYS_VNODE_READDIR as u32,
    });

    let saved_rsp: u64;
    // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
    // SAFETY: saved_rsp is valid.
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// `SYS_VNODE_SEEK(fd, offset, whence)` — seek within a file.
///
/// `SEEK_SET` and `SEEK_CUR` are handled locally without a server roundtrip.
/// `SEEK_END` returns `ENOSYS` for now (requires stat for file size).
#[expect(
    clippy::cast_possible_wrap,
    reason = "seek offset fits in isize on x86_64"
)]
pub fn sys_vnode_seek(fd: usize, offset: usize, whence: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);
    let offset_val = offset as i64;

    let result: Result<i64, hadron_objects::handle::HandleError> = super::with_handle_table(|ht| {
        let entry = ht.get(hv)?;
        let vnode = entry
            .object()
            .as_any()
            .downcast_ref::<Vnode>()
            .ok_or(hadron_objects::handle::HandleError::NotFound)?;

        match whence as u32 {
            SEEK_SET => {
                if offset_val < 0 {
                    return Ok(-EINVAL as i64);
                }
                vnode.set_seek_offset(offset_val as u64);
                Ok(offset_val)
            }
            SEEK_CUR => {
                let cur = vnode.seek_offset() as i64;
                let new_off = cur.saturating_add(offset_val);
                if new_off < 0 {
                    return Ok(-EINVAL as i64);
                }
                vnode.set_seek_offset(new_off as u64);
                Ok(new_off)
            }
            SEEK_END => {
                // Would need a stat roundtrip — deferred.
                Ok(-ENOSYS as i64)
            }
            _ => Ok(-EINVAL as i64),
        }
    });

    match result {
        Ok(v) => v as isize,
        Err(_) => -EBADF,
    }
}

/// `SYS_VNODE_UNLINK` — stub.
pub fn sys_vnode_unlink(_path_ptr: usize, _path_len: usize) -> isize {
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

/// `SYS_VFS_MOUNT(prefix_ptr, prefix_len, channel_fd)` — mount a filesystem.
///
/// Registers a mount point in the global VfsRouter and adds the entry to the
/// calling process's namespace.
#[expect(clippy::cast_possible_truncation, reason = "channel_fd fits in u32")]
pub fn sys_vfs_mount(prefix_ptr: usize, prefix_len: usize, channel_fd: usize) -> isize {
    let prefix_slice = match UserSlice::new(prefix_ptr, prefix_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: User buffer was range-validated.
    let prefix_bytes = unsafe { prefix_slice.read_to_vec() };
    let prefix = match core::str::from_utf8(&prefix_bytes) {
        Ok(p) => p,
        Err(_) => return -EINVAL,
    };

    let hv = HandleValue::from_raw(channel_fd as u32);

    // Look up the channel handle and clone the Arc.
    let channel: Arc<Channel> = match super::with_handle_table(|ht| {
        let entry = ht.get_with_rights(hv, Rights::READ | Rights::WRITE)?;
        if entry.object().object_type() != hadron_objects::object::ObjectType::Channel {
            return Err(hadron_objects::handle::HandleError::NotFound);
        }
        // Clone the Arc<dyn KernelObject> and downcast.
        Ok(Arc::clone(entry.object()))
    }) {
        Ok(obj) => {
            // SAFETY: We verified the type above.
            let ptr = Arc::into_raw(obj).cast::<Channel>();
            unsafe { Arc::from_raw(ptr) }
        }
        Err(_) => return -EBADF,
    };

    let ch_koid = channel.koid();

    // Register in the global mount table.
    if let Err(e) = vfs::with(|router| router.mount(prefix, channel)) {
        return match e {
            vfs::VfsError::AlreadyMounted => -EEXIST,
            _ => -EINVAL,
        };
    }

    // Add to process namespace.
    process::with_current_process(|p| {
        p.namespace_add(alloc::string::String::from(prefix), ch_koid);
    });

    crate::kinfo!("vfs", "mounted \"{}\"", prefix);
    0
}

/// `SYS_VFS_UNMOUNT(prefix_ptr, prefix_len)` — unmount a filesystem.
pub fn sys_vfs_unmount(prefix_ptr: usize, prefix_len: usize) -> isize {
    let prefix_slice = match UserSlice::new(prefix_ptr, prefix_len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: User buffer was range-validated.
    let prefix_bytes = unsafe { prefix_slice.read_to_vec() };
    let prefix = match core::str::from_utf8(&prefix_bytes) {
        Ok(p) => p,
        Err(_) => return -EINVAL,
    };

    if let Err(e) = vfs::with(|router| router.unmount(prefix)) {
        return match e {
            vfs::VfsError::NotFound => -ENOENT,
            _ => -EINVAL,
        };
    }

    process::with_current_process(|p| {
        p.namespace_remove(prefix);
    });

    crate::kinfo!("vfs", "unmounted \"{}\"", prefix);
    0
}
