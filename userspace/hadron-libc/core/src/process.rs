//! Process management functions.
//!
//! POSIX functions: `_exit`, `exit`, `getpid`, `getppid`, `waitpid`,
//! `execve`, `kill`, `getcwd`, `chdir`.

use crate::errno;
use crate::sys;

/// Terminate the process immediately without running atexit handlers.
///
/// # Safety
///
/// This function does not return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _exit(status: i32) -> ! {
    sys::sys_exit(status as usize)
}

/// Terminate the process after running atexit handlers and flushing stdio.
///
/// # Safety
///
/// This function does not return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn exit(status: i32) -> ! {
    // Run atexit handlers in LIFO order.
    crate::atexit::run_handlers();

    // Flush all stdio streams.
    unsafe { crate::stdio::fflush(core::ptr::null_mut()) };

    sys::sys_exit(status as usize)
}

/// Return the process ID of the calling process.
#[unsafe(no_mangle)]
pub extern "C" fn getpid() -> i32 {
    sys::sys_getpid() as i32
}

/// Return the parent process ID.
#[unsafe(no_mangle)]
pub extern "C" fn getppid() -> i32 {
    sys::sys_getppid() as i32
}

/// Wait for a child process to change state.
///
/// # Safety
///
/// `status` must be null or point to a valid `i32`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32 {
    // Hadron waitpid uses a u64 status buffer.
    let mut raw_status: u64 = 0;
    let status_ptr = if status.is_null() {
        core::ptr::null_mut()
    } else {
        &raw mut raw_status
    };

    match sys::sys_waitpid(pid as usize, status_ptr, options as usize) {
        Ok(ret) => {
            if !status.is_null() {
                // SAFETY: Caller guarantees status is valid.
                unsafe { *status = raw_status as i32 };
            }
            ret as i32
        }
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Send a signal to a process.
#[unsafe(no_mangle)]
pub extern "C" fn kill(pid: i32, sig: i32) -> i32 {
    match sys::sys_kill(pid as usize, sig as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Get the current working directory.
///
/// # Safety
///
/// `buf` must be valid for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getcwd(buf: *mut u8, size: usize) -> *mut u8 {
    if buf.is_null() || size == 0 {
        errno::set_errno(crate::errno::EINVAL);
        return core::ptr::null_mut();
    }
    match sys::sys_getcwd(buf, size) {
        Ok(_len) => buf,
        Err(e) => {
            errno::set_errno(e);
            core::ptr::null_mut()
        }
    }
}

/// Change the current working directory.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn chdir(path: *const u8) -> i32 {
    if path.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    // SAFETY: path is NUL-terminated.
    let len = unsafe { crate::string::strlen(path) };
    let slice = unsafe { core::slice::from_raw_parts(path, len) };
    match sys::sys_chdir(slice) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Set process group ID.
#[unsafe(no_mangle)]
pub extern "C" fn setpgid(pid: i32, pgid: i32) -> i32 {
    match sys::sys_setpgid(pid as usize, pgid as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Get process group ID.
#[unsafe(no_mangle)]
pub extern "C" fn getpgid(pid: i32) -> i32 {
    match sys::sys_getpgid(pid as usize) {
        Ok(pgid) => pgid as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Create a new session.
#[unsafe(no_mangle)]
pub extern "C" fn setsid() -> i32 {
    match sys::sys_setsid() {
        Ok(sid) => sid as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}
