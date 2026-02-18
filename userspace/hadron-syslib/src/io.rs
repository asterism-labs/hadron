//! I/O primitives: `read`, `write`, and `print!`/`println!` macros.

use hadron_syscall::raw::{syscall1, syscall3};
use hadron_syscall::{
    DirEntryInfo, SYS_HANDLE_CLOSE, SYS_VNODE_OPEN, SYS_VNODE_READ, SYS_VNODE_READDIR,
    SYS_VNODE_STAT, SYS_VNODE_WRITE, StatInfo,
};

// ── File descriptor constants ─────────────────────────────────────────

/// Standard input file descriptor.
pub const STDIN: usize = 0;
/// Standard output file descriptor.
pub const STDOUT: usize = 1;
/// Standard error file descriptor.
pub const STDERR: usize = 2;

// ── Raw I/O functions ─────────────────────────────────────────────────

/// Read from a file descriptor into `buf`. Returns bytes read or negative errno.
pub fn read(fd: usize, buf: &mut [u8]) -> isize {
    syscall3(SYS_VNODE_READ, fd, buf.as_mut_ptr() as usize, buf.len())
}

/// Write `buf` to a file descriptor. Returns bytes written or negative errno.
pub fn write(fd: usize, buf: &[u8]) -> isize {
    syscall3(SYS_VNODE_WRITE, fd, buf.as_ptr() as usize, buf.len())
}

/// Open a file by path. Returns a file descriptor or negative errno.
pub fn open(path: &str, flags: usize) -> isize {
    syscall3(SYS_VNODE_OPEN, path.as_ptr() as usize, path.len(), flags)
}

/// Close a file descriptor. Returns 0 on success or negative errno.
pub fn close(fd: usize) -> isize {
    syscall1(SYS_HANDLE_CLOSE, fd)
}

/// Stat a file descriptor. Returns `Some(StatInfo)` on success, or `None`.
pub fn stat(fd: usize) -> Option<StatInfo> {
    let mut info = core::mem::MaybeUninit::<StatInfo>::uninit();
    let ret = syscall3(
        SYS_VNODE_STAT,
        fd,
        info.as_mut_ptr() as usize,
        core::mem::size_of::<StatInfo>(),
    );
    if ret >= 0 {
        // SAFETY: The kernel wrote a valid StatInfo into the buffer on success.
        Some(unsafe { info.assume_init() })
    } else {
        None
    }
}

/// Read directory entries from a directory fd.
///
/// Returns the number of entries read into `buf`, or a negative errno.
/// Each entry is a [`DirEntryInfo`].
pub fn readdir(fd: usize, buf: &mut [DirEntryInfo]) -> isize {
    let byte_len = buf.len() * core::mem::size_of::<DirEntryInfo>();
    syscall3(SYS_VNODE_READDIR, fd, buf.as_mut_ptr() as usize, byte_len)
}

// ── fmt::Write implementation for stdout/stderr ───────────────────────

/// A writer that sends bytes to a specific file descriptor.
struct FdWriter(usize);

impl core::fmt::Write for FdWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let result = write(self.0, s.as_bytes());
        if result < 0 {
            Err(core::fmt::Error)
        } else {
            Ok(())
        }
    }
}

/// Internal: write formatted arguments to stdout.
#[doc(hidden)]
pub fn _print(args: core::fmt::Arguments<'_>) {
    use core::fmt::Write;
    let _ = FdWriter(STDOUT).write_fmt(args);
}

/// Internal: write formatted arguments to stderr.
#[doc(hidden)]
pub fn _eprint(args: core::fmt::Arguments<'_>) {
    use core::fmt::Write;
    let _ = FdWriter(STDERR).write_fmt(args);
}

/// Print to standard output.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::io::_print(format_args!($($arg)*))
    };
}

/// Print to standard output with a trailing newline.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {
        $crate::io::_print(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// Print to standard error.
#[macro_export]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::io::_eprint(format_args!($($arg)*))
    };
}

/// Print to standard error with a trailing newline.
#[macro_export]
macro_rules! eprintln {
    () => { $crate::eprint!("\n") };
    ($($arg:tt)*) => {
        $crate::io::_eprint(format_args!("{}\n", format_args!($($arg)*)))
    };
}
