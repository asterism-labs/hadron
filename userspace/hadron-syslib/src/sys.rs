//! System calls: exit, query, clock.

use crate::syscall::{syscall1, syscall4, syscall2};

// ── Syscall numbers ───────────────────────────────────────────────────

const SYS_TASK_EXIT: usize = 0x00;
const SYS_CLOCK_GETTIME: usize = 0x54;
const SYS_QUERY: usize = 0xF0;

// ── Query topics ──────────────────────────────────────────────────────

const QUERY_MEMORY: u64 = 0;
const QUERY_UPTIME: u64 = 1;
const QUERY_KERNEL_VERSION: u64 = 2;

// ── Clock IDs ─────────────────────────────────────────────────────────

const CLOCK_MONOTONIC: usize = 0;

// ── Data structures (must match hadron-core layout) ───────────────────

/// POSIX-compatible timespec.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Timespec {
    /// Seconds since boot.
    pub tv_sec: u64,
    /// Nanoseconds within the current second.
    pub tv_nsec: u64,
}

/// Physical memory statistics.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryInfo {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Free physical memory in bytes.
    pub free_bytes: u64,
    /// Used physical memory in bytes.
    pub used_bytes: u64,
}

/// Time since boot.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct UptimeInfo {
    /// Nanoseconds since boot.
    pub uptime_ns: u64,
}

/// Kernel version metadata.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KernelVersionInfo {
    /// Major version.
    pub major: u16,
    /// Minor version.
    pub minor: u16,
    /// Patch version.
    pub patch: u16,
    /// Padding.
    pub _pad: u16,
    /// Kernel name (UTF-8, NUL-padded).
    pub name: [u8; 32],
}

// ── Functions ─────────────────────────────────────────────────────────

/// Terminate the current process with the given exit status.
pub fn exit(status: usize) -> ! {
    syscall1(SYS_TASK_EXIT, status);
    // The kernel should never return from exit, but just in case:
    loop {
        // SAFETY: hlt is safe in ring 3 (it's a no-op that yields to the OS),
        // but the kernel will have already killed this process.
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)) };
    }
}

/// Query physical memory statistics.
pub fn query_memory() -> Option<MemoryInfo> {
    let mut info = core::mem::MaybeUninit::<MemoryInfo>::uninit();
    let ret = syscall4(
        SYS_QUERY,
        QUERY_MEMORY as usize,
        info.as_mut_ptr() as usize,
        core::mem::size_of::<MemoryInfo>(),
        0,
    );
    if ret >= 0 {
        // SAFETY: The kernel wrote a valid MemoryInfo into the buffer on success.
        Some(unsafe { info.assume_init() })
    } else {
        None
    }
}

/// Query time since boot.
pub fn query_uptime() -> Option<UptimeInfo> {
    let mut info = core::mem::MaybeUninit::<UptimeInfo>::uninit();
    let ret = syscall4(
        SYS_QUERY,
        QUERY_UPTIME as usize,
        info.as_mut_ptr() as usize,
        core::mem::size_of::<UptimeInfo>(),
        0,
    );
    if ret >= 0 {
        // SAFETY: The kernel wrote a valid UptimeInfo into the buffer on success.
        Some(unsafe { info.assume_init() })
    } else {
        None
    }
}

/// Query kernel version information.
pub fn query_kernel_version() -> Option<KernelVersionInfo> {
    let mut info = core::mem::MaybeUninit::<KernelVersionInfo>::uninit();
    let ret = syscall4(
        SYS_QUERY,
        QUERY_KERNEL_VERSION as usize,
        info.as_mut_ptr() as usize,
        core::mem::size_of::<KernelVersionInfo>(),
        0,
    );
    if ret >= 0 {
        // SAFETY: The kernel wrote a valid KernelVersionInfo into the buffer on success.
        Some(unsafe { info.assume_init() })
    } else {
        None
    }
}

/// Get the current monotonic time.
pub fn clock_gettime() -> Option<Timespec> {
    let mut ts = core::mem::MaybeUninit::<Timespec>::uninit();
    let ret = syscall2(
        SYS_CLOCK_GETTIME,
        CLOCK_MONOTONIC,
        ts.as_mut_ptr() as usize,
    );
    if ret >= 0 {
        // SAFETY: The kernel wrote a valid Timespec into the buffer on success.
        Some(unsafe { ts.assume_init() })
    } else {
        None
    }
}
