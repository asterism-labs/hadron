//! I/O primitives: `read`, `write`, and `print!`/`println!` macros.

use crate::syscall::{syscall3, syscall1};

// ── Syscall numbers (duplicated from hadron-core for independence) ────

/// `SYS_VNODE_READ` — read from a file descriptor.
const SYS_VNODE_READ: usize = 0x31;
/// `SYS_VNODE_WRITE` — write to a file descriptor.
const SYS_VNODE_WRITE: usize = 0x32;
/// `SYS_VNODE_OPEN` — open a file by path.
const SYS_VNODE_OPEN: usize = 0x30;

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

/// Close a file descriptor (not yet implemented in kernel, provided for future use).
pub fn close(fd: usize) -> isize {
    // SYS_HANDLE_CLOSE = 0x10
    syscall1(0x10, fd)
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
