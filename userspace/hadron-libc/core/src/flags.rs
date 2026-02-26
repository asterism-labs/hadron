//! POSIX-to-Hadron flag translation.
//!
//! Hadron uses its own flag encoding for syscalls. This module translates
//! between standard POSIX constants and Hadron's internal values.

// ---- POSIX open flags (standard Linux values) --------------------------------

pub const O_RDONLY: u32 = 0;
pub const O_WRONLY: u32 = 1;
pub const O_RDWR: u32 = 2;
pub const O_CREAT: u32 = 0o100;
pub const O_EXCL: u32 = 0o200;
pub const O_NOFOLLOW: u32 = 0o400000;
pub const O_TRUNC: u32 = 0o1000;
pub const O_APPEND: u32 = 0o2000;
pub const O_NONBLOCK: u32 = 0o4000;
pub const O_DIRECTORY: u32 = 0o200000;
pub const O_CLOEXEC: u32 = 0o2000000;

// ---- Hadron internal open flags (from hadron-syscall) -------------------------

const HADRON_OPEN_READ: usize = 0x0001;
const HADRON_OPEN_WRITE: usize = 0x0002;
const HADRON_OPEN_CREATE: usize = 0x0004;
const HADRON_OPEN_TRUNCATE: usize = 0x0008;
const HADRON_OPEN_APPEND: usize = 0x0010;
const HADRON_OPEN_CLOEXEC: usize = 0x0020;
const HADRON_OPEN_NONBLOCK: usize = 0x0040;
const HADRON_OPEN_DIRECTORY: usize = 0x0080;
const HADRON_OPEN_EXCL: usize = 0x0100;
const HADRON_OPEN_NOFOLLOW: usize = 0x0200;

/// Translate POSIX `open()` flags to Hadron internal flags.
pub fn posix_open_to_hadron(flags: u32) -> usize {
    let access = flags & 0x3; // O_RDONLY=0, O_WRONLY=1, O_RDWR=2
    let mut out: usize = match access {
        0 => HADRON_OPEN_READ,
        1 => HADRON_OPEN_WRITE,
        2 => HADRON_OPEN_READ | HADRON_OPEN_WRITE,
        _ => HADRON_OPEN_READ,
    };
    if flags & O_CREAT != 0 {
        out |= HADRON_OPEN_CREATE;
    }
    if flags & O_TRUNC != 0 {
        out |= HADRON_OPEN_TRUNCATE;
    }
    if flags & O_APPEND != 0 {
        out |= HADRON_OPEN_APPEND;
    }
    if flags & O_CLOEXEC != 0 {
        out |= HADRON_OPEN_CLOEXEC;
    }
    if flags & O_NONBLOCK != 0 {
        out |= HADRON_OPEN_NONBLOCK;
    }
    if flags & O_DIRECTORY != 0 {
        out |= HADRON_OPEN_DIRECTORY;
    }
    if flags & O_EXCL != 0 {
        out |= HADRON_OPEN_EXCL;
    }
    if flags & O_NOFOLLOW != 0 {
        out |= HADRON_OPEN_NOFOLLOW;
    }
    out
}

// ---- POSIX mmap flags --------------------------------------------------------

pub const MAP_SHARED: u32 = 0x01;
pub const MAP_PRIVATE: u32 = 0x02;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_FAILED: *mut u8 = usize::MAX as *mut u8;

pub const PROT_NONE: u32 = 0x0;
pub const PROT_READ: u32 = 0x1;
pub const PROT_WRITE: u32 = 0x2;
pub const PROT_EXEC: u32 = 0x4;

// Hadron mmap flags
const HADRON_MAP_ANONYMOUS: usize = 0x1;
const HADRON_MAP_SHARED: usize = 0x2;

/// Translate POSIX mmap flags to Hadron internal flags.
pub fn posix_mmap_to_hadron(flags: u32) -> usize {
    let mut out: usize = 0;
    if flags & MAP_ANONYMOUS != 0 {
        out |= HADRON_MAP_ANONYMOUS;
    }
    if flags & MAP_SHARED != 0 {
        out |= HADRON_MAP_SHARED;
    }
    out
}

/// Translate POSIX mmap prot to Hadron prot (same values).
pub fn posix_prot_to_hadron(prot: u32) -> usize {
    prot as usize
}

// ---- Seek (same values) ------------------------------------------------------

pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;

// ---- fcntl commands (same values) --------------------------------------------

pub const F_DUPFD: i32 = 0;
pub const F_GETFD: i32 = 1;
pub const F_SETFD: i32 = 2;
pub const F_GETFL: i32 = 3;
pub const F_SETFL: i32 = 4;
pub const F_DUPFD_CLOEXEC: i32 = 0x406;
pub const FD_CLOEXEC: i32 = 1;

// ---- Signal numbers (same as Linux) ------------------------------------------

pub const SIGHUP: i32 = 1;
pub const SIGINT: i32 = 2;
pub const SIGQUIT: i32 = 3;
pub const SIGILL: i32 = 4;
pub const SIGABRT: i32 = 6;
pub const SIGFPE: i32 = 8;
pub const SIGKILL: i32 = 9;
pub const SIGSEGV: i32 = 11;
pub const SIGPIPE: i32 = 13;
pub const SIGALRM: i32 = 14;
pub const SIGTERM: i32 = 15;
pub const SIGCHLD: i32 = 17;
pub const SIGCONT: i32 = 18;
pub const SIGSTOP: i32 = 19;
pub const SIGTSTP: i32 = 20;
pub const SIGTTIN: i32 = 21;
pub const SIGTTOU: i32 = 22;
pub const SIGUSR1: i32 = 10;
pub const SIGUSR2: i32 = 12;

pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;
pub const SA_RESTART: usize = 0x1000_0000;
pub const SA_RESETHAND: usize = 0x8000_0000;

// ---- Signal mask operations --------------------------------------------------

pub const SIG_BLOCK: i32 = 0;
pub const SIG_UNBLOCK: i32 = 1;
pub const SIG_SETMASK: i32 = 2;

// ---- Wait flags --------------------------------------------------------------

pub const WNOHANG: i32 = 1;
pub const WUNTRACED: i32 = 2;

// ---- Clock IDs ---------------------------------------------------------------

pub const CLOCK_MONOTONIC: i32 = 0;
pub const CLOCK_REALTIME: i32 = 1;

// ---- Poll events -------------------------------------------------------------

pub const POLLIN: i16 = 0x0001;
pub const POLLOUT: i16 = 0x0004;
pub const POLLERR: i16 = 0x0008;
pub const POLLHUP: i16 = 0x0010;
pub const POLLNVAL: i16 = 0x0020;

// ---- Pipe flags --------------------------------------------------------------

pub const PIPE_CLOEXEC: u32 = 0x0020;
pub const PIPE_NONBLOCK: u32 = 0x0040;

// ---- File type macros --------------------------------------------------------

pub const S_IFMT: u32 = 0o170000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFLNK: u32 = 0o120000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_open_rdonly() {
        assert_eq!(posix_open_to_hadron(O_RDONLY), 0x0001);
    }

    #[test]
    fn posix_open_rdwr_create_trunc() {
        let flags = O_RDWR | O_CREAT | O_TRUNC;
        let hadron = posix_open_to_hadron(flags);
        assert_eq!(hadron & 0x0001, 0x0001); // READ
        assert_eq!(hadron & 0x0002, 0x0002); // WRITE
        assert_eq!(hadron & 0x0004, 0x0004); // CREATE
        assert_eq!(hadron & 0x0008, 0x0008); // TRUNCATE
    }

    #[test]
    fn posix_mmap_anon_private() {
        let hadron = posix_mmap_to_hadron(MAP_ANONYMOUS | MAP_PRIVATE);
        assert_eq!(hadron, 0x1); // only ANONYMOUS, PRIVATE has no hadron equivalent
    }

    #[test]
    fn posix_mmap_shared() {
        let hadron = posix_mmap_to_hadron(MAP_SHARED);
        assert_eq!(hadron, 0x2);
    }
}
