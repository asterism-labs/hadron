//! Shared `#[repr(C)]` types passed between userspace and the kernel.
//!
//! All types in this module are layout-stable so they can be safely
//! copied across the syscall boundary.

/// Information about physical memory, returned by [`SYS_QUERY`] with
/// [`QUERY_MEMORY`](super::constants::QUERY_MEMORY).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct MemoryInfo {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Currently free physical memory in bytes.
    pub free_bytes: u64,
    /// Memory used by the kernel in bytes.
    pub kernel_bytes: u64,
}

/// Time since boot, returned by [`SYS_QUERY`] with
/// [`QUERY_UPTIME`](super::constants::QUERY_UPTIME).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct UptimeInfo {
    /// Seconds since boot.
    pub secs: u64,
    /// Nanoseconds within the current second.
    pub nanos: u64,
}

/// Kernel version information, returned by [`SYS_QUERY`] with
/// [`QUERY_KERNEL_VERSION`](super::constants::QUERY_KERNEL_VERSION).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct KernelVersionInfo {
    /// Major version number.
    pub major: u32,
    /// Minor version number.
    pub minor: u32,
    /// Patch version number.
    pub patch: u32,
    /// Padding for alignment.
    pub _pad: u32,
}

/// Process table information, returned by [`SYS_QUERY`] with
/// [`QUERY_PROCESSES`](super::constants::QUERY_PROCESSES).
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct ProcessInfo {
    /// Number of running processes.
    pub process_count: u32,
    /// Number of running threads.
    pub thread_count: u32,
}

/// POSIX-compatible time specification.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Timespec {
    /// Seconds.
    pub tv_sec: u64,
    /// Nanoseconds (0..999_999_999).
    pub tv_nsec: u64,
}

/// A string descriptor for passing argv/envp entries across the syscall
/// boundary.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SpawnArg {
    /// Pointer to the string data in user memory.
    pub ptr: usize,
    /// Length of the string in bytes (not null-terminated).
    pub len: usize,
}

/// File descriptor mapping entry for [`SYS_TASK_SPAWN`].
///
/// Maps a parent handle to a specific slot in the child's handle table.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FdMapEntry {
    /// Handle slot in the child process.
    pub child_fd: u32,
    /// Handle value in the parent process to duplicate.
    pub parent_fd: u32,
}

/// Process spawn descriptor passed to [`SYS_TASK_SPAWN`].
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct SpawnInfo {
    /// Pointer to the binary path string.
    pub path_ptr: usize,
    /// Length of the path string.
    pub path_len: usize,
    /// Pointer to an array of [`SpawnArg`] for argv.
    pub argv_ptr: usize,
    /// Number of argv entries.
    pub argv_count: usize,
    /// Pointer to an array of [`SpawnArg`] for envp.
    pub envp_ptr: usize,
    /// Number of envp entries.
    pub envp_count: usize,
    /// Pointer to an array of [`FdMapEntry`] for handle inheritance.
    pub fd_map_ptr: usize,
    /// Number of fd_map entries.
    pub fd_map_count: usize,
    /// Pointer to the initial working directory path (0 = inherit).
    pub cwd_ptr: usize,
    /// Length of the working directory path.
    pub cwd_len: usize,
}

/// Poll file descriptor for [`SYS_EVENT_WAIT_MANY`].
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct PollFd {
    /// Handle (file descriptor) to poll.
    pub fd: u32,
    /// Requested events (bitmask of `POLL*` constants).
    pub events: u16,
    /// Returned events (filled by kernel).
    pub revents: u16,
}
