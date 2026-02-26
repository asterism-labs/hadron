//! Low-level I/O functions (POSIX file descriptor layer).
//!
//! POSIX functions: `open`, `close`, `read`, `write`, `lseek`,
//! `dup`, `dup2`, `pipe`, `pipe2`, `fcntl`, `ioctl`, `stat`, `fstat`, `isatty`.

use crate::errno;
use crate::sys;

/// Open a file.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn open(path: *const u8, flags: i32) -> i32 {
    if path.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // SAFETY: path is NUL-terminated.
    let len = unsafe { crate::string::strlen(path) };
    let slice = unsafe { core::slice::from_raw_parts(path, len) };
    let hadron_flags = crate::flags::posix_open_to_hadron(flags as u32);
    match sys::sys_open(slice, hadron_flags) {
        Ok(fd) => fd as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Close a file descriptor.
#[unsafe(no_mangle)]
pub extern "C" fn close(fd: i32) -> i32 {
    match sys::sys_close(fd as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Read from a file descriptor.
///
/// # Safety
///
/// `buf` must be valid for `count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn read(fd: i32, buf: *mut u8, count: usize) -> isize {
    if buf.is_null() && count > 0 {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // SAFETY: Caller guarantees buf valid for count bytes.
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, count) };
    match sys::sys_read(fd as usize, slice) {
        Ok(n) => n as isize,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Write to a file descriptor.
///
/// # Safety
///
/// `buf` must be valid for `count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn write(fd: i32, buf: *const u8, count: usize) -> isize {
    if buf.is_null() && count > 0 {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // SAFETY: Caller guarantees buf valid for count bytes.
    let slice = unsafe { core::slice::from_raw_parts(buf, count) };
    match sys::sys_write(fd as usize, slice) {
        Ok(n) => n as isize,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Reposition read/write file offset.
#[unsafe(no_mangle)]
pub extern "C" fn lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    match sys::sys_lseek(fd as usize, offset, whence as usize) {
        Ok(pos) => pos as i64,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Duplicate a file descriptor to the lowest available fd.
#[unsafe(no_mangle)]
pub extern "C" fn dup(fd: i32) -> i32 {
    match sys::sys_dup_lowest(fd as usize) {
        Ok(new_fd) => new_fd as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Duplicate a file descriptor to a specific fd.
#[unsafe(no_mangle)]
pub extern "C" fn dup2(oldfd: i32, newfd: i32) -> i32 {
    match sys::sys_dup(oldfd as usize, newfd as usize) {
        Ok(fd) => fd as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Create a pipe.
///
/// # Safety
///
/// `fds` must point to an array of two `i32`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe(fds: *mut i32) -> i32 {
    if fds.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let mut raw_fds: [usize; 2] = [0; 2];
    match sys::sys_pipe(&raw mut raw_fds) {
        Ok(()) => {
            // SAFETY: fds points to at least 2 i32s.
            unsafe {
                *fds = raw_fds[0] as i32;
                *fds.add(1) = raw_fds[1] as i32;
            }
            0
        }
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Create a pipe with flags.
///
/// # Safety
///
/// `fds` must point to an array of two `i32`s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pipe2(fds: *mut i32, flags: i32) -> i32 {
    if fds.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let mut raw_fds: [usize; 2] = [0; 2];
    match sys::sys_pipe2(&raw mut raw_fds, flags as usize) {
        Ok(()) => {
            // SAFETY: fds points to at least 2 i32s.
            unsafe {
                *fds = raw_fds[0] as i32;
                *fds.add(1) = raw_fds[1] as i32;
            }
            0
        }
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// File control operations.
#[unsafe(no_mangle)]
pub extern "C" fn fcntl(fd: i32, cmd: i32, arg: usize) -> i32 {
    match sys::sys_fcntl(fd as usize, cmd as usize, arg) {
        Ok(ret) => ret as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Device control operations.
#[unsafe(no_mangle)]
pub extern "C" fn ioctl(fd: i32, cmd: u64, arg: usize) -> i32 {
    match sys::sys_ioctl(fd as usize, cmd as usize, arg) {
        Ok(ret) => ret as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Check if a file descriptor refers to a terminal.
#[unsafe(no_mangle)]
pub extern "C" fn isatty(fd: i32) -> i32 {
    // Probe with TCGETS ioctl (0x5401). If it succeeds, it's a terminal.
    const TCGETS: usize = 0x5401;
    let mut termios_buf = [0u8; 64]; // Enough for a termios struct.
    match sys::sys_ioctl(fd as usize, TCGETS, termios_buf.as_mut_ptr() as usize) {
        Ok(_) => 1,
        Err(e) => {
            errno::set_errno(e);
            0
        }
    }
}

/// Get file status by path.
///
/// # Safety
///
/// `path` must be NUL-terminated. `buf` must be valid for stat struct size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn stat(path: *const u8, buf: *mut u8) -> i32 {
    if path.is_null() || buf.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // Open path, stat the fd, close.
    let fd = unsafe { open(path, crate::flags::O_RDONLY as i32) };
    if fd < 0 {
        return -1;
    }
    let ret = unsafe { fstat(fd, buf) };
    close(fd);
    ret
}

/// Get file status by file descriptor.
///
/// # Safety
///
/// `buf` must be valid for the kernel's stat struct size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fstat(fd: i32, buf: *mut u8) -> i32 {
    if buf.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // Use a generous buffer size for the kernel stat struct.
    const STAT_BUF_SIZE: usize = 128;
    match sys::sys_stat(fd as usize, buf, STAT_BUF_SIZE) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Remove a file.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unlink(path: *const u8) -> i32 {
    if path.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // SAFETY: path is NUL-terminated.
    let len = unsafe { crate::string::strlen(path) };
    let slice = unsafe { core::slice::from_raw_parts(path, len) };
    match sys::sys_unlink(slice) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Create a directory.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mkdir(path: *const u8, mode: u32) -> i32 {
    if path.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // SAFETY: path is NUL-terminated.
    let len = unsafe { crate::string::strlen(path) };
    let slice = unsafe { core::slice::from_raw_parts(path, len) };
    match sys::sys_mkdir(slice, mode as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}
