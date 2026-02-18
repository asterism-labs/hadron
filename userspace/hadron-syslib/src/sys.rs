//! System calls: exit, query, clock.

use hadron_syscall::raw::{syscall1, syscall2, syscall4};
use hadron_syscall::{
    CLOCK_MONOTONIC, KernelVersionInfo, MemoryInfo, QUERY_KERNEL_VERSION, QUERY_MEMORY,
    QUERY_UPTIME, SYS_CLOCK_GETTIME, SYS_QUERY, SYS_TASK_EXIT, Timespec, UptimeInfo,
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
