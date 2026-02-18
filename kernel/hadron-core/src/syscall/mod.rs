//! Native Hadron syscall number constants and user-space pointer validation.
//!
//! Grouped numbering with room for future expansion per category.

pub mod userptr;

// ── Task management (0x00–0x0F) ──────────────────────────────────────

/// Terminate the current task.
pub const SYS_TASK_EXIT: usize = 0x00;
/// Spawn a new task (Phase 9+).
#[allow(dead_code, reason = "reserved for Phase 9+")]
pub const SYS_TASK_SPAWN: usize = 0x01;
/// Wait for a task to exit (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_TASK_WAIT: usize = 0x02;
/// Kill a task (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_TASK_KILL: usize = 0x03;
/// Detach a task (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_TASK_DETACH: usize = 0x04;
/// Query task information (returns task ID for now).
pub const SYS_TASK_INFO: usize = 0x05;

// ── Handle operations (0x10–0x1F) ────────────────────────────────────

/// Close a handle (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_HANDLE_CLOSE: usize = 0x10;
/// Duplicate a handle (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_HANDLE_DUP: usize = 0x11;
/// Query handle info (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_HANDLE_INFO: usize = 0x12;

// ── Channels (0x20–0x2F) ─────────────────────────────────────────────

/// Create a channel pair (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_CHANNEL_CREATE: usize = 0x20;
/// Send a message on a channel (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_CHANNEL_SEND: usize = 0x21;
/// Receive a message from a channel (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_CHANNEL_RECV: usize = 0x22;
/// Synchronous call on a channel (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_CHANNEL_CALL: usize = 0x23;

// ── Filesystem / vnodes (0x30–0x3F) ──────────────────────────────────

/// Open a vnode.
pub const SYS_VNODE_OPEN: usize = 0x30;
/// Read from a vnode.
pub const SYS_VNODE_READ: usize = 0x31;
/// Write to a vnode.
pub const SYS_VNODE_WRITE: usize = 0x32;
/// Stat a vnode (Phase 10+).
#[allow(dead_code, reason = "reserved for Phase 10+")]
pub const SYS_VNODE_STAT: usize = 0x33;
/// Read directory entries (Phase 10+).
#[allow(dead_code, reason = "reserved for Phase 10+")]
pub const SYS_VNODE_READDIR: usize = 0x34;
/// Unlink a vnode (Phase 10+).
#[allow(dead_code, reason = "reserved for Phase 10+")]
pub const SYS_VNODE_UNLINK: usize = 0x35;

// ── Memory (0x40–0x4F) ───────────────────────────────────────────────

/// Map memory into the address space.
pub const SYS_MEM_MAP: usize = 0x40;
/// Unmap memory from the address space.
pub const SYS_MEM_UNMAP: usize = 0x41;
/// Change memory protection flags.
#[allow(dead_code, reason = "reserved for Phase 9+")]
pub const SYS_MEM_PROTECT: usize = 0x42;
/// Create a shared memory object (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_MEM_CREATE_SHARED: usize = 0x43;
/// Map a shared memory object (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_MEM_MAP_SHARED: usize = 0x44;

// ── Events and time (0x50–0x5F) ──────────────────────────────────────

/// Create an event object (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_EVENT_CREATE: usize = 0x50;
/// Signal an event (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_EVENT_SIGNAL: usize = 0x51;
/// Wait for an event (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_EVENT_WAIT: usize = 0x52;
/// Wait for multiple events (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_EVENT_WAIT_MANY: usize = 0x53;
/// Get current time.
pub const SYS_CLOCK_GETTIME: usize = 0x54;
/// Create a timer (Phase 11).
#[allow(dead_code, reason = "reserved for Phase 11")]
pub const SYS_TIMER_CREATE: usize = 0x55;

// ── System (0xF0–0xFF) ───────────────────────────────────────────────

/// Query system information via typed `#[repr(C)]` response structs.
pub const SYS_QUERY: usize = 0xF0;
/// Write a debug message to the kernel serial console.
pub const SYS_DEBUG_LOG: usize = 0xF1;

// ── Query topics for SYS_QUERY ──────────────────────────────────────

/// Query topic: physical memory statistics.
pub const QUERY_MEMORY: u64 = 0;
/// Query topic: time since boot.
pub const QUERY_UPTIME: u64 = 1;
/// Query topic: kernel version information.
pub const QUERY_KERNEL_VERSION: u64 = 2;

// ── Clock IDs ───────────────────────────────────────────────────────

/// Monotonic clock: nanoseconds since boot, never adjusted.
pub const CLOCK_MONOTONIC: usize = 0;

// ── Error numbers ────────────────────────────────────────────────────

/// `ENOENT` — no such file or directory.
pub const ENOENT: isize = 2;
/// `EIO` — I/O error.
pub const EIO: isize = 5;
/// `EBADF` — bad file descriptor.
pub const EBADF: isize = 9;
/// `EACCES` — permission denied.
pub const EACCES: isize = 13;
/// `EFAULT` — bad address.
pub const EFAULT: isize = 14;
/// `EEXIST` — file exists.
pub const EEXIST: isize = 17;
/// `ENOTDIR` — not a directory.
pub const ENOTDIR: isize = 20;
/// `EISDIR` — is a directory.
pub const EISDIR: isize = 21;
/// `EINVAL` — invalid argument.
pub const EINVAL: isize = 22;
/// `ENOSYS` — function not implemented.
pub const ENOSYS: isize = 38;

// ── Syscall data structures ─────────────────────────────────────────

/// POSIX-compatible timespec for `clock_gettime` results.
///
/// Uses `u64` fields (not `i64`) because Hadron only supports monotonic
/// boot-relative time — negative timestamps are impossible.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Timespec {
    /// Seconds since boot.
    pub tv_sec: u64,
    /// Nanoseconds within the current second (0..999_999_999).
    pub tv_nsec: u64,
}

/// Response for [`QUERY_MEMORY`]: physical memory statistics.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryInfo {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Free physical memory in bytes.
    pub free_bytes: u64,
    /// Used physical memory in bytes (`total_bytes - free_bytes`).
    pub used_bytes: u64,
}

/// Response for [`QUERY_UPTIME`]: time elapsed since boot.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UptimeInfo {
    /// Nanoseconds since boot.
    pub uptime_ns: u64,
}

/// Response for [`QUERY_KERNEL_VERSION`]: kernel version metadata.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KernelVersionInfo {
    /// Major version number.
    pub major: u16,
    /// Minor version number.
    pub minor: u16,
    /// Patch version number.
    pub patch: u16,
    /// Padding for alignment.
    pub _pad: u16,
    /// Kernel name as a UTF-8 byte array, NUL-padded.
    pub name: [u8; 32],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syscall_numbers_unique() {
        // All active (non-dead-code) syscall numbers must be distinct.
        let active = [
            SYS_TASK_EXIT,
            SYS_TASK_INFO,
            SYS_VNODE_OPEN,
            SYS_VNODE_READ,
            SYS_VNODE_WRITE,
            SYS_MEM_MAP,
            SYS_MEM_UNMAP,
            SYS_CLOCK_GETTIME,
            SYS_QUERY,
            SYS_DEBUG_LOG,
        ];
        for (i, a) in active.iter().enumerate() {
            for (j, b) in active.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "syscall numbers at index {i} and {j} collide");
                }
            }
        }
    }

    #[test]
    fn error_numbers_nonzero() {
        assert!(ENOENT > 0);
        assert!(EIO > 0);
        assert!(EBADF > 0);
        assert!(EACCES > 0);
        assert!(EFAULT > 0);
        assert!(EEXIST > 0);
        assert!(ENOTDIR > 0);
        assert!(EISDIR > 0);
        assert!(EINVAL > 0);
        assert!(ENOSYS > 0);
    }

    #[test]
    fn syscall_categories_non_overlapping() {
        // Task management: 0x00–0x0F
        // Handle operations: 0x10–0x1F
        // Channels: 0x20–0x2F
        // Filesystem: 0x30–0x3F
        // Memory: 0x40–0x4F
        // Events/time: 0x50–0x5F
        // System: 0xF0–0xFF
        assert!(SYS_TASK_EXIT < 0x10);
        assert!(SYS_TASK_INFO < 0x10);
        assert!((0x10..0x20).contains(&SYS_HANDLE_CLOSE));
        assert!((0x20..0x30).contains(&SYS_CHANNEL_CREATE));
        assert!((0x30..0x40).contains(&SYS_VNODE_OPEN));
        assert!((0x40..0x50).contains(&SYS_MEM_MAP));
        assert!((0x50..0x60).contains(&SYS_CLOCK_GETTIME));
        assert!((0xF0..=0xFF).contains(&SYS_QUERY));
        assert!((0xF0..=0xFF).contains(&SYS_DEBUG_LOG));
    }
}
