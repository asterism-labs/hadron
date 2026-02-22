//! Syscall tests — raw `syscall` instruction verification.

use hadron_ktest::kernel_test;

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_syscall_task_info() {
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") crate::syscall::SYS_TASK_INFO,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    assert!(result >= 0, "sys_task_info returned {}", result);
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_syscall_unknown_returns_enosys() {
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 9999usize,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    assert_eq!(
        result,
        -(crate::syscall::ENOSYS),
        "unknown syscall should return -ENOSYS"
    );
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_syscall_debug_log() {
    let msg = b"syscall debug_log test\n";
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") crate::syscall::SYS_DEBUG_LOG,
            in("rdi") msg.as_ptr() as usize,
            in("rsi") msg.len(),
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    assert_eq!(
        result,
        msg.len() as isize,
        "sys_debug_log should return len"
    );
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_syscall_clock_gettime() {
    // Use sentinel values to confirm the syscall overwrites the struct.
    let mut ts = crate::syscall::Timespec {
        tv_sec: u64::MAX,
        tv_nsec: u64::MAX,
    };
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") crate::syscall::SYS_CLOCK_GETTIME,
            in("rdi") crate::syscall::CLOCK_MONOTONIC,
            in("rsi") &mut ts as *mut _ as usize,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
        );
    }
    assert_eq!(result, 0, "clock_gettime should succeed");
    assert_ne!(ts.tv_nsec, u64::MAX, "tv_nsec should be overwritten");
    assert!(ts.tv_nsec < 1_000_000_000, "tv_nsec must be < 1 billion");
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_syscall_clock_gettime_invalid_clock() {
    let mut ts = crate::syscall::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") crate::syscall::SYS_CLOCK_GETTIME,
            in("rdi") 99usize, // invalid clock ID
            in("rsi") &mut ts as *mut _ as usize,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
        );
    }
    assert_eq!(
        result,
        -(crate::syscall::EINVAL),
        "invalid clock should return -EINVAL"
    );
}
