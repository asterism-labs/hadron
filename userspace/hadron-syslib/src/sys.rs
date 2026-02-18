//! System calls: exit, getpid, spawn, waitpid, pipe, query, clock.

use hadron_syscall::raw::{syscall0, syscall1, syscall2, syscall4};
use hadron_syscall::{
    CLOCK_MONOTONIC, KernelVersionInfo, MemoryInfo, QUERY_KERNEL_VERSION, QUERY_MEMORY,
    QUERY_UPTIME, SYS_CLOCK_GETTIME, SYS_HANDLE_DUP, SYS_HANDLE_PIPE, SYS_QUERY, SYS_TASK_EXIT,
    SYS_TASK_INFO, SYS_TASK_SPAWN, SYS_TASK_WAIT, Timespec, UptimeInfo,
};

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

/// Get the current process ID.
#[expect(
    clippy::cast_possible_truncation,
    reason = "PIDs fit in u32; isize is sufficient"
)]
pub fn getpid() -> u32 {
    syscall0(SYS_TASK_INFO) as u32
}

/// Spawn a new process from an ELF binary at the given path.
///
/// Returns the child PID on success, or a negative errno on failure.
pub fn spawn(path: &str) -> isize {
    syscall2(SYS_TASK_SPAWN, path.as_ptr() as usize, path.len())
}

/// Wait for a child process to exit.
///
/// If `pid` is 0, waits for any child. Returns the child PID on success.
/// If `status_out` is `Some`, the child's exit status is written there.
pub fn waitpid(pid: u32, status_out: Option<&mut u64>) -> isize {
    let status_ptr = match status_out {
        Some(s) => s as *mut u64 as usize,
        None => 0,
    };
    syscall2(SYS_TASK_WAIT, pid as usize, status_ptr)
}

/// Duplicate a file descriptor (dup2 semantics).
///
/// Copies `old_fd` to `new_fd`, closing `new_fd` first if it was open.
/// Returns `new_fd` on success, or a negative errno on failure.
pub fn dup2(old_fd: usize, new_fd: usize) -> isize {
    syscall2(SYS_HANDLE_DUP, old_fd, new_fd)
}

/// Create a pipe. Returns `(read_fd, write_fd)` on success.
pub fn pipe() -> Result<(usize, usize), isize> {
    let mut fds: [usize; 2] = [0; 2];
    let ret = syscall1(SYS_HANDLE_PIPE, fds.as_mut_ptr() as usize);
    if ret < 0 {
        Err(ret)
    } else {
        Ok((fds[0], fds[1]))
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
    let ret = syscall2(SYS_CLOCK_GETTIME, CLOCK_MONOTONIC, ts.as_mut_ptr() as usize);
    if ret >= 0 {
        // SAFETY: The kernel wrote a valid Timespec into the buffer on success.
        Some(unsafe { ts.assume_init() })
    } else {
        None
    }
}
