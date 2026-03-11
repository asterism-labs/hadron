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
