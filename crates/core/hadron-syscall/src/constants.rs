//! Flag constants shared between kernel and userspace.

// ── Query types ──────────────────────────────────────────────────────

/// Query physical memory statistics.
pub const QUERY_MEMORY: u32 = 1;
/// Query time since boot.
pub const QUERY_UPTIME: u32 = 2;
/// Query kernel version.
pub const QUERY_KERNEL_VERSION: u32 = 3;
/// Query process table statistics.
pub const QUERY_PROCESSES: u32 = 4;

// ── Clock IDs ────────────────────────────────────────────────────────

/// Monotonic clock (cannot be set, not affected by NTP).
pub const CLOCK_MONOTONIC: u32 = 1;
/// Real-time (wall clock) clock.
pub const CLOCK_REALTIME: u32 = 0;

// ── Memory protection flags ──────────────────────────────────────────

/// Pages may be read.
pub const PROT_READ: usize = 0x1;
/// Pages may be written.
pub const PROT_WRITE: usize = 0x2;
/// Pages may be executed.
pub const PROT_EXEC: usize = 0x4;
/// Pages may not be accessed.
pub const PROT_NONE: usize = 0x0;

// ── Memory mapping flags ─────────────────────────────────────────────

/// Changes are shared with other mappings.
pub const MAP_SHARED: usize = 0x01;
/// Changes are private (copy-on-write).
pub const MAP_PRIVATE: usize = 0x02;
/// Use the address hint exactly.
pub const MAP_FIXED: usize = 0x10;
/// Mapping is not backed by a file (anonymous).
pub const MAP_ANONYMOUS: usize = 0x20;

// ── Open flags ───────────────────────────────────────────────────────

/// Open for reading only.
pub const OPEN_RDONLY: u32 = 0;
/// Open for writing only.
pub const OPEN_WRONLY: u32 = 1;
/// Open for reading and writing.
pub const OPEN_RDWR: u32 = 2;
/// Create file if it does not exist.
pub const OPEN_CREAT: u32 = 0x40;
/// Truncate file to zero length.
pub const OPEN_TRUNC: u32 = 0x200;
/// Append to file.
pub const OPEN_APPEND: u32 = 0x400;
/// Open directory.
pub const OPEN_DIRECTORY: u32 = 0x10000;

// ── Signal numbers ───────────────────────────────────────────────────

/// Interrupt from keyboard (Ctrl-C).
pub const SIGINT: usize = 2;
/// Quit from keyboard (Ctrl-\\).
pub const SIGQUIT: usize = 3;
/// Kill signal (cannot be caught or ignored).
pub const SIGKILL: usize = 9;
/// Invalid memory reference.
pub const SIGSEGV: usize = 11;
/// Broken pipe.
pub const SIGPIPE: usize = 13;
/// Termination signal.
pub const SIGTERM: usize = 15;
/// Child stopped or terminated.
pub const SIGCHLD: usize = 17;
/// Stop process (cannot be caught or ignored).
pub const SIGSTOP: usize = 19;

/// Default signal action.
pub const SIG_DFL: usize = 0;
/// Ignore signal.
pub const SIG_IGN: usize = 1;

// ── Futex operations ─────────────────────────────────────────────────

/// Block if `*addr == expected`.
pub const FUTEX_WAIT: u32 = 0;
/// Wake up to `val` waiters.
pub const FUTEX_WAKE: u32 = 1;

// ── Seek whence constants ───────────────────────────────────────────

/// Seek from the beginning of the file.
pub const SEEK_SET: u32 = 0;
/// Seek from the current position.
pub const SEEK_CUR: u32 = 1;
/// Seek from the end of the file.
pub const SEEK_END: u32 = 2;

// ── Poll event flags ─────────────────────────────────────────────────

/// Data available for reading.
pub const POLLIN: u16 = 0x001;
/// Writing is possible.
pub const POLLOUT: u16 = 0x004;
/// Error condition.
pub const POLLERR: u16 = 0x008;
/// Hang up (peer closed).
pub const POLLHUP: u16 = 0x010;
/// Invalid request (fd not open).
pub const POLLNVAL: u16 = 0x020;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prot_flags_no_overlap() {
        let flags: &[usize] = &[PROT_READ, PROT_WRITE, PROT_EXEC];
        for (i, a) in flags.iter().enumerate() {
            for b in &flags[i + 1..] {
                assert_eq!(a & b, 0, "PROT flags overlap: {a:#x} and {b:#x}");
            }
        }
    }

    #[test]
    fn map_flags_no_overlap() {
        let flags: &[usize] = &[MAP_SHARED, MAP_PRIVATE, MAP_FIXED, MAP_ANONYMOUS];
        for (i, a) in flags.iter().enumerate() {
            for b in &flags[i + 1..] {
                assert_eq!(a & b, 0, "MAP flags overlap: {a:#x} and {b:#x}");
            }
        }
    }

    #[test]
    fn open_flags_no_overlap() {
        // Access modes (low 2 bits) are allowed to overlap — test creation/mode flags only.
        let flags: &[u32] = &[OPEN_CREAT, OPEN_TRUNC, OPEN_APPEND, OPEN_DIRECTORY];
        for (i, a) in flags.iter().enumerate() {
            for b in &flags[i + 1..] {
                assert_eq!(a & b, 0, "OPEN flags overlap: {a:#x} and {b:#x}");
            }
        }
    }

    #[test]
    fn poll_flags_no_overlap() {
        let flags: &[u16] = &[POLLIN, POLLOUT, POLLERR, POLLHUP, POLLNVAL];
        for (i, a) in flags.iter().enumerate() {
            for b in &flags[i + 1..] {
                assert_eq!(a & b, 0, "POLL flags overlap: {a:#x} and {b:#x}");
            }
        }
    }

    #[test]
    fn query_types_unique() {
        let types: &[u32] = &[
            QUERY_MEMORY,
            QUERY_UPTIME,
            QUERY_KERNEL_VERSION,
            QUERY_PROCESSES,
        ];
        for (i, a) in types.iter().enumerate() {
            for (j, b) in types.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "duplicate query type {a} at indices {i} and {j}");
                }
            }
        }
    }

    #[test]
    fn signal_numbers_unique() {
        let sigs: &[usize] = &[
            SIGINT, SIGQUIT, SIGKILL, SIGSEGV, SIGPIPE, SIGTERM, SIGCHLD, SIGSTOP,
        ];
        for (i, a) in sigs.iter().enumerate() {
            for (j, b) in sigs.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "duplicate signal number {a} at indices {i} and {j}");
                }
            }
        }
    }
}
