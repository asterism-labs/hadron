//! POSIX errno management and error constants.
//!
//! Provides a thread-local errno value (Phase 1: global `AtomicI32`, no TCB)
//! and POSIX-compatible error number constants matching the kernel's values.

use core::sync::atomic::{AtomicI32, Ordering};

/// Newtype wrapper for POSIX error numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct Errno(pub i32);

// Phase 1: single static (no TCB/TLS). Correct for single-threaded processes.
static ERRNO: AtomicI32 = AtomicI32::new(0);

/// Set the global errno value.
#[inline]
pub fn set_errno(e: Errno) {
    ERRNO.store(e.0, Ordering::Relaxed);
}

/// Get the current errno value.
#[inline]
pub fn get_errno() -> Errno {
    Errno(ERRNO.load(Ordering::Relaxed))
}

// POSIX error constants — values match the kernel's hadron_syscall error codes.
pub const EPERM: Errno = Errno(1);
pub const ENOENT: Errno = Errno(2);
pub const ESRCH: Errno = Errno(3);
pub const EINTR: Errno = Errno(4);
pub const EIO: Errno = Errno(5);
pub const ENXIO: Errno = Errno(6);
pub const E2BIG: Errno = Errno(7);
pub const ENOEXEC: Errno = Errno(8);
pub const EBADF: Errno = Errno(9);
pub const ECHILD: Errno = Errno(10);
pub const EAGAIN: Errno = Errno(11);
pub const ENOMEM: Errno = Errno(12);
pub const EACCES: Errno = Errno(13);
pub const EFAULT: Errno = Errno(14);
pub const EBUSY: Errno = Errno(16);
pub const EEXIST: Errno = Errno(17);
pub const EXDEV: Errno = Errno(18);
pub const ENODEV: Errno = Errno(19);
pub const ENOTDIR: Errno = Errno(20);
pub const EISDIR: Errno = Errno(21);
pub const EINVAL: Errno = Errno(22);
pub const ENFILE: Errno = Errno(23);
pub const EMFILE: Errno = Errno(24);
pub const ENOTTY: Errno = Errno(25);
pub const EFBIG: Errno = Errno(27);
pub const ENOSPC: Errno = Errno(28);
pub const ESPIPE: Errno = Errno(29);
pub const EROFS: Errno = Errno(30);
pub const EPIPE: Errno = Errno(32);
pub const EDOM: Errno = Errno(33);
pub const ERANGE: Errno = Errno(34);
pub const ENAMETOOLONG: Errno = Errno(36);
pub const ENOSYS: Errno = Errno(38);
pub const ENOTEMPTY: Errno = Errno(39);
pub const ELOOP: Errno = Errno(40);
pub const EWOULDBLOCK: Errno = EAGAIN;
pub const EMSGSIZE: Errno = Errno(90);
pub const ENOTSOCK: Errno = Errno(88);
pub const EAFNOSUPPORT: Errno = Errno(97);
pub const EADDRINUSE: Errno = Errno(98);
pub const ENOTCONN: Errno = Errno(107);
pub const EISCONN: Errno = Errno(106);
pub const ECONNREFUSED: Errno = Errno(111);

/// C ABI: `int *__errno_location(void)` — returns pointer to errno storage.
///
/// # Safety
///
/// The returned pointer is valid for the lifetime of the process. Phase 1 uses
/// a single global atomic, so concurrent mutation from multiple threads is safe
/// at the atomic level but not semantically correct (deferred to Phase 2 with TLS).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __errno_location() -> *mut i32 {
    ERRNO.as_ptr()
}
