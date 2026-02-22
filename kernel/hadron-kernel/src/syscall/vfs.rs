//! VFS syscall handlers: open, read, write, close, stat, readdir, dup.

use crate::id::Fd;
use crate::syscall::EFAULT;
use crate::syscall::userptr::UserSlice;

use crate::fs::file::OpenFlags;
use crate::fs::{poll_immediate, try_poll_immediate};

/// `sys_vnode_open` — open a file by path, returning a file descriptor.
///
/// Arguments:
/// - `path_ptr`: user-space pointer to the path string
/// - `path_len`: length of the path string
/// - `flags`: open flags (bitwise OR of `OpenFlags` values)
///
/// Returns a non-negative fd on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_vnode_open(path_ptr: usize, path_len: usize, flags: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(path_ptr, path_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [path_ptr, path_ptr+path_len) is in user space.
    let path_bytes = unsafe { user_slice.as_slice() };
    let Ok(path) = core::str::from_utf8(path_bytes) else {
        return -crate::syscall::EINVAL;
    };

    #[expect(clippy::cast_possible_truncation, reason = "open flags fit in u32")]
    let open_flags = OpenFlags::from_bits_truncate(flags as u32);

    // Resolve path via VFS.
    let Ok(inode) = crate::fs::vfs::with_vfs(|vfs| vfs.resolve(path)) else {
        return -crate::syscall::ENOENT;
    };

    // Allocate fd in the current process's fd table.
    let fd = crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        fd_table.open(inode, open_flags)
    });

    fd.as_u32() as isize
}

/// `sys_vnode_read` — read from an open file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to the destination buffer
/// - `buf_len`: maximum number of bytes to read
///
/// Returns the number of bytes read on success, or a negative errno on failure.
/// If the underlying I/O would block (e.g. pipe with no data), triggers
/// TRAP_IO to handle the async read in `process_task`.
#[expect(
    clippy::cast_possible_wrap,
    reason = "byte counts are small, wrap is impossible"
)]
pub(super) fn sys_vnode_read(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_mut_slice() };

    // Extract inode and offset from fd table, then release the process lock
    // before performing I/O. This is critical: trap_io() does a longjmp and
    // must never be called while holding the CURRENT_PROCESS spinlock.
    let (inode, offset) = match crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return Err(-crate::syscall::EBADF);
        };

        if !file.flags.contains(OpenFlags::READ) {
            return Err(-crate::syscall::EBADF);
        }

        Ok((file.inode.clone(), file.offset))
    }) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    match try_poll_immediate(inode.read(offset, buf)) {
        Some(Ok(n)) => {
            // Re-acquire to update offset.
            crate::proc::ProcessTable::with_current(|process| {
                let mut fd_table = process.fd_table.lock();
                if let Some(f) = fd_table.get_mut(fd) {
                    f.offset += n;
                }
            });
            n as isize
        }
        Some(Err(e)) => -e.to_errno(),
        None => {
            // I/O would block — trap to process_task for async handling.
            // This is outside with_current_process, so the longjmp is safe.
            trap_io(fd, buf_ptr, buf_len, false)
        }
    }
}

/// `sys_vnode_write` — write to an open file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to the source buffer
/// - `buf_len`: number of bytes to write
///
/// Returns the number of bytes written on success, or a negative errno on failure.
/// If the underlying I/O would block (e.g. pipe with full buffer), triggers
/// TRAP_IO to handle the async write in `process_task`.
#[expect(
    clippy::cast_possible_wrap,
    reason = "byte counts are small, wrap is impossible"
)]
pub(super) fn sys_vnode_write(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    let Ok(user_slice) = UserSlice::new(buf_ptr, buf_len) else {
        return -EFAULT;
    };

    // SAFETY: UserSlice validated that [buf_ptr, buf_ptr+buf_len) is in user space.
    let buf = unsafe { user_slice.as_slice() };

    // Extract inode and offset from fd table, then release the process lock
    // before performing I/O. This is critical: trap_io() does a longjmp and
    // must never be called while holding the CURRENT_PROCESS spinlock.
    let (inode, offset) = match crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return Err(-crate::syscall::EBADF);
        };

        if !file.flags.contains(OpenFlags::WRITE) {
            return Err(-crate::syscall::EBADF);
        }

        Ok((file.inode.clone(), file.offset))
    }) {
        Ok(pair) => pair,
        Err(e) => return e,
    };

    match try_poll_immediate(inode.write(offset, buf)) {
        Some(Ok(n)) => {
            // Re-acquire to update offset.
            crate::proc::ProcessTable::with_current(|process| {
                let mut fd_table = process.fd_table.lock();
                if let Some(f) = fd_table.get_mut(fd) {
                    f.offset += n;
                }
            });
            n as isize
        }
        Some(Err(e)) => -e.to_errno(),
        None => {
            // I/O would block — trap to process_task for async handling.
            // This is outside with_current_process, so the longjmp is safe.
            trap_io(fd, buf_ptr, buf_len, true)
        }
    }
}

/// `sys_handle_close` — close a file descriptor.
///
/// Arguments:
/// - `fd`: file descriptor number
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_handle_close(fd: usize) -> isize {
    let fd = Fd::new(fd as u32);
    crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        match fd_table.close(fd) {
            Ok(()) => 0,
            Err(e) => -e.to_errno(),
        }
    })
}

/// `sys_handle_dup` — duplicate a file descriptor (dup2 semantics).
///
/// Arguments:
/// - `old_fd`: source file descriptor
/// - `new_fd`: destination file descriptor (closed silently if already open)
///
/// Returns `new_fd` on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_handle_dup(old_fd: usize, new_fd: usize) -> isize {
    let old_fd = Fd::new(old_fd as u32);
    let new_fd = Fd::new(new_fd as u32);
    crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        let Some(src) = fd_table.get(old_fd) else {
            return -crate::syscall::EBADF;
        };
        let inode = src.inode.clone();
        let flags = src.flags;

        // If new_fd is already open, close it silently (POSIX dup2 semantics).
        let _ = fd_table.close(new_fd);

        fd_table.insert_at(new_fd, inode, flags);
        new_fd.as_u32() as isize
    })
}

/// `sys_vnode_stat` — get file status information.
///
/// Arguments:
/// - `fd`: file descriptor number
/// - `buf_ptr`: user-space pointer to write [`StatInfo`] to
/// - `buf_len`: size of the user buffer (must be >= `size_of::<StatInfo>()`)
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_vnode_stat(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    use crate::syscall::{EINVAL, StatInfo};

    let stat_size = core::mem::size_of::<StatInfo>();
    if buf_len < stat_size {
        return -EINVAL;
    }

    let Ok(user_slice) = UserSlice::new(buf_ptr, stat_size) else {
        return -EFAULT;
    };

    crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return -crate::syscall::EBADF;
        };

        let inode = file.inode.clone();
        drop(fd_table);

        let inode_type = match inode.inode_type() {
            crate::fs::InodeType::File => crate::syscall::INODE_TYPE_FILE,
            crate::fs::InodeType::Directory => crate::syscall::INODE_TYPE_DIR,
            crate::fs::InodeType::CharDevice => crate::syscall::INODE_TYPE_CHARDEV,
            crate::fs::InodeType::Symlink => crate::syscall::INODE_TYPE_SYMLINK,
        };

        let perms = inode.permissions();
        let permissions: u32 =
            u32::from(perms.read) | (u32::from(perms.write) << 1) | (u32::from(perms.execute) << 2);

        let info = StatInfo {
            inode_type,
            _pad: [0; 3],
            size: inode.size() as u64,
            permissions,
        };

        // SAFETY: UserSlice validated the pointer range is in user space,
        // and we write exactly stat_size bytes.
        let out = unsafe { user_slice.as_mut_slice() };
        // SAFETY: StatInfo is repr(C) and contains only scalar fields.
        let info_bytes = unsafe {
            core::slice::from_raw_parts(core::ptr::addr_of!(info).cast::<u8>(), stat_size)
        };
        out[..stat_size].copy_from_slice(info_bytes);
        0
    })
}

/// `sys_handle_pipe` — create a pipe and return [read_fd, write_fd].
///
/// Arguments:
/// - `fds_ptr`: user-space pointer to write two `usize` values (read_fd, write_fd)
///
/// Returns 0 on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "fd numbers are small, wrap is impossible"
)]
pub(super) fn sys_handle_pipe(fds_ptr: usize) -> isize {
    let Ok(user_slice) = UserSlice::new(fds_ptr, 2 * core::mem::size_of::<usize>()) else {
        return -EFAULT;
    };

    let (reader, writer) = crate::ipc::pipe::pipe();

    let (read_fd, write_fd) = crate::proc::ProcessTable::with_current(|process| {
        let mut fd_table = process.fd_table.lock();
        let rfd = fd_table.open(reader, OpenFlags::READ);
        let wfd = fd_table.open(writer, OpenFlags::WRITE);
        (rfd, wfd)
    });

    // SAFETY: UserSlice validated the pointer range is in user space.
    // The ABI returns fd numbers as usize values to userspace.
    unsafe {
        let dst = user_slice.addr() as *mut usize;
        core::ptr::write(dst, read_fd.as_usize());
        core::ptr::write(dst.add(1), write_fd.as_usize());
    }

    0
}

/// `sys_vnode_readdir` — read directory entries.
///
/// Arguments:
/// - `fd`: file descriptor number (must refer to a directory)
/// - `buf_ptr`: user-space pointer to write [`DirEntryInfo`] array to
/// - `buf_len`: size of the user buffer in bytes
///
/// Returns the number of entries written on success, or a negative errno on failure.
#[expect(
    clippy::cast_possible_wrap,
    reason = "entry counts are small, wrap is impossible"
)]
pub(super) fn sys_vnode_readdir(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let fd = Fd::new(fd as u32);
    use crate::syscall::DirEntryInfo;

    let entry_size = core::mem::size_of::<DirEntryInfo>();
    let max_entries = buf_len / entry_size;
    if max_entries == 0 {
        return -crate::syscall::EINVAL;
    }

    let total_bytes = max_entries * entry_size;
    let Ok(user_slice) = UserSlice::new(buf_ptr, total_bytes) else {
        return -EFAULT;
    };

    crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return -crate::syscall::EBADF;
        };

        let inode = file.inode.clone();
        drop(fd_table);

        let entries = match poll_immediate(inode.readdir()) {
            Ok(entries) => entries,
            Err(e) => return -e.to_errno(),
        };

        // SAFETY: UserSlice validated the pointer range is in user space.
        let out = unsafe { user_slice.as_mut_slice() };
        let mut written = 0;

        for entry in &entries {
            if written >= max_entries {
                break;
            }

            let inode_type = match entry.inode_type {
                crate::fs::InodeType::File => crate::syscall::INODE_TYPE_FILE,
                crate::fs::InodeType::Directory => crate::syscall::INODE_TYPE_DIR,
                crate::fs::InodeType::CharDevice => crate::syscall::INODE_TYPE_CHARDEV,
                crate::fs::InodeType::Symlink => crate::syscall::INODE_TYPE_SYMLINK,
            };

            let name_bytes = entry.name.as_bytes();
            let name_len = name_bytes.len().min(60);

            let mut info = DirEntryInfo {
                inode_type,
                name_len: name_len as u8,
                _pad: [0; 2],
                name: [0; 60],
            };
            info.name[..name_len].copy_from_slice(&name_bytes[..name_len]);

            // SAFETY: DirEntryInfo is repr(C) and contains only scalar fields.
            let info_bytes = unsafe {
                core::slice::from_raw_parts(core::ptr::addr_of!(info).cast::<u8>(), entry_size)
            };
            let offset = written * entry_size;
            out[offset..offset + entry_size].copy_from_slice(info_bytes);
            written += 1;
        }

        written as isize
    })
}

/// `sys_handle_tcsetpgrp` — set the foreground process group of a TTY.
///
/// Currently operates on the active TTY regardless of `fd`, since all
/// terminal fds (/dev/console, /dev/tty0) point to the same TTY.
///
/// Returns 0 on success, or a negative errno on failure.
pub(super) fn sys_handle_tcsetpgrp(_fd: usize, pgid: usize) -> isize {
    let pgid = pgid as u32;
    crate::tty::active_tty().set_foreground_pgid(pgid);
    0
}

/// `sys_handle_tcgetpgrp` — get the foreground process group of a TTY.
///
/// Currently operates on the active TTY regardless of `fd`.
///
/// Returns the PGID on success (0 if none set), or a negative errno.
#[expect(
    clippy::cast_possible_wrap,
    reason = "PGIDs are small positive integers, wrap is impossible"
)]
pub(super) fn sys_handle_tcgetpgrp(_fd: usize) -> isize {
    crate::tty::active_tty()
        .foreground_pgid()
        .unwrap_or(0) as isize
}

/// Trigger a TRAP_IO longjmp back to `process_task` for async I/O.
///
/// Sets the I/O parameters, restores kernel CR3 and GS bases, then
/// calls `restore_kernel_context` — never returns.
fn trap_io(fd: Fd, buf_ptr: usize, buf_len: usize, is_write: bool) -> ! {
    use crate::arch::x86_64::registers::control::Cr3;
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
    use crate::arch::x86_64::userspace::restore_kernel_context;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();

    // SAFETY: Restoring kernel CR3 and GS bases is the standard pattern
    // for returning from userspace context to kernel context.
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    crate::proc::IoState::set_params(fd, buf_ptr, buf_len, is_write);
    crate::proc::TrapContext::set_trap_reason(crate::proc::TrapReason::Io);

    let saved_rsp = crate::proc::TrapContext::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}
