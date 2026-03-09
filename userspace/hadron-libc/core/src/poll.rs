//! POSIX `poll()` — I/O multiplexing.
//!
//! `struct pollfd` has the same memory layout as Hadron's `PollFd`:
//! `{fd: u32, events: u16, revents: u16}` = 8 bytes, identical to POSIX
//! `{int fd; short events; short revents;}`. The pointer is passed directly
//! to `event_wait_many` without any translation.

use crate::{errno, sys};

/// Poll file descriptors for I/O readiness.
///
/// `fds` is a pointer to an array of `struct pollfd`. `nfds` is the count.
/// `timeout` is in milliseconds: negative = wait forever, 0 = non-blocking,
/// positive = timeout in milliseconds.
///
/// Returns the number of ready file descriptors on success, 0 on timeout,
/// or -1 on error (with errno set).
///
/// # Safety
///
/// `fds` must point to a valid array of `nfds` `struct pollfd` descriptors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn poll(fds: *mut u8, nfds: usize, timeout: i32) -> i32 {
    // Map POSIX timeout semantics to Hadron:
    // POSIX: <0 = infinite, 0 = non-blocking, >0 = ms
    // Hadron: usize::MAX = infinite, 0 = non-blocking, n = ms
    let timeout_ms: isize = if timeout < 0 { -1 } else { timeout as isize };
    match sys::sys_poll(fds, nfds, timeout_ms) {
        Ok(n) => n as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}
