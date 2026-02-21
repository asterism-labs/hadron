//! System calls: exit, getpid, spawn, waitpid, pipe, query, clock.

use hadron_syscall::raw::{syscall0, syscall1, syscall2, syscall4};
use hadron_syscall::{
    CLOCK_MONOTONIC, KernelVersionInfo, MAP_ANONYMOUS, MemoryInfo, PROT_READ, PROT_WRITE,
    QUERY_KERNEL_VERSION, QUERY_MEMORY, QUERY_UPTIME, SYS_CLOCK_GETTIME, SYS_HANDLE_DUP,
    SYS_HANDLE_PIPE, SYS_MEM_MAP, SYS_MEM_UNMAP, SYS_QUERY, SYS_TASK_EXIT, SYS_TASK_INFO,
    SYS_TASK_SPAWN, SYS_TASK_WAIT, SpawnArg, Timespec, UptimeInfo,
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
/// `argv` is passed to the child process. If empty, no arguments are passed.
///
/// Returns the child PID on success, or a negative errno on failure.
pub fn spawn(path: &str, argv: &[&str]) -> isize {
    if argv.is_empty() {
        return syscall4(SYS_TASK_SPAWN, path.as_ptr() as usize, path.len(), 0, 0);
    }

    // Build SpawnArg descriptors on the stack.
    // Max 32 args to match the kernel limit.
    let mut descs = [SpawnArg { ptr: 0, len: 0 }; 32];
    let count = argv.len().min(32);
    for (i, arg) in argv[..count].iter().enumerate() {
        descs[i] = SpawnArg {
            ptr: arg.as_ptr() as usize,
            len: arg.len(),
        };
    }

    syscall4(
        SYS_TASK_SPAWN,
        path.as_ptr() as usize,
        path.len(),
        descs.as_ptr() as usize,
        count,
    )
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
        0, // sub_id (reserved)
        info.as_mut_ptr() as usize,
        core::mem::size_of::<MemoryInfo>(),
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
        0, // sub_id (reserved)
        info.as_mut_ptr() as usize,
        core::mem::size_of::<UptimeInfo>(),
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
        0, // sub_id (reserved)
        info.as_mut_ptr() as usize,
        core::mem::size_of::<KernelVersionInfo>(),
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

/// Map anonymous read-write memory into the process address space.
///
/// Returns a pointer to the mapped region, or `None` if the mapping failed.
/// The returned pointer is page-aligned. `length` is rounded up to page size.
pub fn mem_map(length: usize) -> Option<*mut u8> {
    let ret = syscall4(
        SYS_MEM_MAP,
        0, // addr_hint (kernel chooses)
        length,
        PROT_READ | PROT_WRITE,
        MAP_ANONYMOUS,
    );
    if ret > 0 {
        Some(ret as *mut u8)
    } else {
        None
    }
}

/// Unmap a previously mapped memory region.
///
/// `addr` must be the exact pointer returned by [`mem_map`]. `length` must
/// match the original request.
///
/// Returns `true` on success.
pub fn mem_unmap(addr: *mut u8, length: usize) -> bool {
    syscall2(SYS_MEM_UNMAP, addr as usize, length) == 0
}
