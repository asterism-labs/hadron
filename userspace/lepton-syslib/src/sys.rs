//! System calls: exit, getpid, spawn, waitpid, kill, pipe, query, clock.

use hadron_syscall::raw::{syscall0, syscall1, syscall2, syscall3, syscall4};
use hadron_syscall::{
    CLOCK_MONOTONIC, KernelVersionInfo, MAP_ANONYMOUS, MemoryInfo, PROT_READ, PROT_WRITE,
    QUERY_KERNEL_VERSION, QUERY_MEMORY, QUERY_UPTIME, SYS_CLOCK_GETTIME, SYS_HANDLE_DUP,
    SYS_HANDLE_PIPE, SYS_MEM_MAP, SYS_MEM_UNMAP, SYS_QUERY, SYS_TASK_EXIT, SYS_TASK_INFO,
    SYS_TASK_KILL, SYS_TASK_SIGACTION, SYS_TASK_SPAWN, SYS_TASK_WAIT, SpawnArg, Timespec,
    UptimeInfo,
};

pub use hadron_syscall::{SIG_DFL, SIG_IGN};

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
/// `argv` is passed to the child process. The current environment is
/// automatically inherited. Returns the child PID on success, or a
/// negative errno on failure.
pub fn spawn(path: &str, argv: &[&str]) -> isize {
    // Build the env block from the current environment.
    let env_block = crate::env::build_env_block();
    let env_refs: alloc::vec::Vec<&str> = env_block.iter().map(|s| s.as_str()).collect();
    spawn_with_env(path, argv, &env_refs)
}

/// Spawn a new process with explicit environment variables.
///
/// Each entry in `envp` should be a `KEY=value` string.
/// Returns the child PID on success, or a negative errno on failure.
pub fn spawn_with_env(path: &str, argv: &[&str], envp: &[&str]) -> isize {
    // Build SpawnArg descriptors for argv.
    let mut argv_descs = [SpawnArg { ptr: 0, len: 0 }; 32];
    let argv_count = argv.len().min(32);
    for (i, arg) in argv[..argv_count].iter().enumerate() {
        argv_descs[i] = SpawnArg {
            ptr: arg.as_ptr() as usize,
            len: arg.len(),
        };
    }

    // Build SpawnArg descriptors for envp.
    let mut envp_descs = [SpawnArg { ptr: 0, len: 0 }; 64];
    let envp_count = envp.len().min(64);
    for (i, env) in envp[..envp_count].iter().enumerate() {
        envp_descs[i] = SpawnArg {
            ptr: env.as_ptr() as usize,
            len: env.len(),
        };
    }

    let info = hadron_syscall::SpawnInfo {
        path_ptr: path.as_ptr() as usize,
        path_len: path.len(),
        argv_ptr: if argv_count > 0 {
            argv_descs.as_ptr() as usize
        } else {
            0
        },
        argv_count,
        envp_ptr: if envp_count > 0 {
            envp_descs.as_ptr() as usize
        } else {
            0
        },
        envp_count,
    };

    syscall2(
        SYS_TASK_SPAWN,
        &info as *const hadron_syscall::SpawnInfo as usize,
        core::mem::size_of::<hadron_syscall::SpawnInfo>(),
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

/// Send a signal to a process.
///
/// Returns 0 on success, or a negative errno on failure.
pub fn kill(pid: u32, signum: usize) -> isize {
    syscall2(SYS_TASK_KILL, pid as usize, signum)
}

/// Register a signal handler for the given signal number.
///
/// `handler` is `SIG_DFL` (0) for default, `SIG_IGN` (1) to ignore, or a
/// function pointer `fn(usize)` cast to `usize`. SIGKILL and SIGSTOP cannot
/// be caught or ignored.
///
/// Returns the previous handler on success, or a negative errno on failure.
pub fn signal(signum: usize, handler: usize) -> isize {
    let mut old_handler: u64 = 0;
    let ret = syscall3(
        SYS_TASK_SIGACTION,
        signum,
        handler,
        &mut old_handler as *mut u64 as usize,
    );
    if ret < 0 {
        ret
    } else {
        old_handler as isize
    }
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
    if ret > 0 { Some(ret as *mut u8) } else { None }
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
