//! Syscall error codes.
//!
//! The kernel returns negated errno values (e.g. `-EFAULT`) from syscalls.
//! Userspace checks `ret < 0` and negates to recover the error code.

/// Operation not permitted.
pub const EPERM: isize = 1;
/// No such file or directory.
pub const ENOENT: isize = 2;
/// No such process.
pub const ESRCH: isize = 3;
/// Interrupted system call.
pub const EINTR: isize = 4;
/// I/O error.
pub const EIO: isize = 5;
/// No such device or address.
pub const ENXIO: isize = 6;
/// Argument list too long.
pub const E2BIG: isize = 7;
/// Exec format error.
pub const ENOEXEC: isize = 8;
/// Bad file descriptor.
pub const EBADF: isize = 9;
/// No child processes.
pub const ECHILD: isize = 10;
/// Resource temporarily unavailable (would block).
pub const EAGAIN: isize = 11;
/// Out of memory.
pub const ENOMEM: isize = 12;
/// Permission denied.
pub const EACCES: isize = 13;
/// Bad address.
pub const EFAULT: isize = 14;
/// Device or resource busy.
pub const EBUSY: isize = 16;
/// File exists.
pub const EEXIST: isize = 17;
/// Invalid cross-device link.
pub const EXDEV: isize = 18;
/// No such device.
pub const ENODEV: isize = 19;
/// Not a directory.
pub const ENOTDIR: isize = 20;
/// Is a directory.
pub const EISDIR: isize = 21;
/// Invalid argument.
pub const EINVAL: isize = 22;
/// Too many open files in system.
pub const ENFILE: isize = 23;
/// Too many open files.
pub const EMFILE: isize = 24;
/// Not a typewriter (inappropriate ioctl).
pub const ENOTTY: isize = 25;
/// File too large.
pub const EFBIG: isize = 27;
/// No space left on device.
pub const ENOSPC: isize = 28;
/// Illegal seek.
pub const ESPIPE: isize = 29;
/// Read-only file system.
pub const EROFS: isize = 30;
/// Broken pipe.
pub const EPIPE: isize = 32;
/// Numerical argument out of range.
pub const ERANGE: isize = 34;
/// Function not implemented.
pub const ENOSYS: isize = 38;
/// Name too long.
pub const ENAMETOOLONG: isize = 36;
/// No message of desired type.
pub const ENOMSG: isize = 42;
/// Connection refused.
pub const ECONNREFUSED: isize = 111;
/// Connection reset by peer.
pub const ECONNRESET: isize = 104;
/// Transport endpoint is not connected.
pub const ENOTCONN: isize = 107;
/// Operation timed out.
pub const ETIMEDOUT: isize = 110;
/// Operation already in progress.
pub const EALREADY: isize = 114;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_errors_positive() {
        let all: &[(&str, isize)] = &[
            ("EPERM", EPERM),
            ("ENOENT", ENOENT),
            ("ESRCH", ESRCH),
            ("EINTR", EINTR),
            ("EIO", EIO),
            ("ENXIO", ENXIO),
            ("E2BIG", E2BIG),
            ("ENOEXEC", ENOEXEC),
            ("EBADF", EBADF),
            ("ECHILD", ECHILD),
            ("EAGAIN", EAGAIN),
            ("ENOMEM", ENOMEM),
            ("EACCES", EACCES),
            ("EFAULT", EFAULT),
            ("EBUSY", EBUSY),
            ("EEXIST", EEXIST),
            ("EXDEV", EXDEV),
            ("ENODEV", ENODEV),
            ("ENOTDIR", ENOTDIR),
            ("EISDIR", EISDIR),
            ("EINVAL", EINVAL),
            ("ENFILE", ENFILE),
            ("EMFILE", EMFILE),
            ("ENOTTY", ENOTTY),
            ("EFBIG", EFBIG),
            ("ENOSPC", ENOSPC),
            ("ESPIPE", ESPIPE),
            ("EROFS", EROFS),
            ("EPIPE", EPIPE),
            ("ERANGE", ERANGE),
            ("ENOSYS", ENOSYS),
            ("ENAMETOOLONG", ENAMETOOLONG),
            ("ENOMSG", ENOMSG),
            ("ECONNREFUSED", ECONNREFUSED),
            ("ECONNRESET", ECONNRESET),
            ("ENOTCONN", ENOTCONN),
            ("ETIMEDOUT", ETIMEDOUT),
            ("EALREADY", EALREADY),
        ];

        for (name, val) in all {
            assert!(*val > 0, "error {name} has non-positive value {val}");
        }
    }

    #[test]
    fn all_errors_unique() {
        let all: &[isize] = &[
            EPERM,
            ENOENT,
            ESRCH,
            EINTR,
            EIO,
            ENXIO,
            E2BIG,
            ENOEXEC,
            EBADF,
            ECHILD,
            EAGAIN,
            ENOMEM,
            EACCES,
            EFAULT,
            EBUSY,
            EEXIST,
            EXDEV,
            ENODEV,
            ENOTDIR,
            EISDIR,
            EINVAL,
            ENFILE,
            EMFILE,
            ENOTTY,
            EFBIG,
            ENOSPC,
            ESPIPE,
            EROFS,
            EPIPE,
            ERANGE,
            ENOSYS,
            ENAMETOOLONG,
            ENOMSG,
            ECONNREFUSED,
            ECONNRESET,
            ENOTCONN,
            ETIMEDOUT,
            EALREADY,
        ];

        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "duplicate error value {a} at indices {i} and {j}");
                }
            }
        }
    }
}
