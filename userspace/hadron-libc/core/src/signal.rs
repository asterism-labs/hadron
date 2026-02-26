//! Signal handling functions.
//!
//! POSIX functions: `sigaction`, `sigprocmask`, `signal`, `raise`.

use crate::errno;
use crate::sys;

/// Default signal handler.
pub const SIG_DFL: usize = 0;
/// Ignore signal.
pub const SIG_IGN: usize = 1;

/// Install a signal action.
///
/// # Safety
///
/// `old_handler` may be null; if non-null, must be valid for one `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigaction(
    signum: i32,
    handler: usize,
    flags: usize,
    old_handler: *mut usize,
) -> i32 {
    match sys::sys_sigaction(signum as usize, handler, flags, old_handler) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Examine or change the signal mask.
///
/// # Safety
///
/// `set` and `oldset` may be null; if non-null, must be valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sigprocmask(how: i32, set: *const u64, oldset: *mut u64) -> i32 {
    match sys::sys_sigprocmask(how as usize, set, oldset) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Simplified signal handler registration (POSIX `signal()`).
///
/// Returns the previous handler, or `SIG_ERR` (usize::MAX) on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn signal(signum: i32, handler: usize) -> usize {
    let mut old: usize = 0;
    match sys::sys_sigaction(signum as usize, handler, 0, &raw mut old) {
        Ok(()) => old,
        Err(e) => {
            errno::set_errno(e);
            usize::MAX // SIG_ERR
        }
    }
}

/// Send a signal to the calling process.
#[unsafe(no_mangle)]
pub extern "C" fn raise(sig: i32) -> i32 {
    let pid = sys::sys_getpid();
    match sys::sys_kill(pid, sig as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}
