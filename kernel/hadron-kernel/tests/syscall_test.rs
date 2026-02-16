#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

hadron_test::test_entry_point_with_init!();

#[test_case]
fn test_syscall_task_info() {
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") hadron_core::syscall::SYS_TASK_INFO,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    assert!(result >= 0, "sys_task_info returned {}", result);
}

#[test_case]
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
        -(hadron_core::syscall::ENOSYS),
        "unknown syscall should return -ENOSYS"
    );
}

#[test_case]
fn test_syscall_debug_log() {
    let msg = b"syscall debug_log test\n";
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") hadron_core::syscall::SYS_DEBUG_LOG,
            in("rdi") msg.as_ptr() as usize,
            in("rsi") msg.len(),
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    assert_eq!(result, msg.len() as isize, "sys_debug_log should return len");
}

#[test_case]
fn test_syscall_clock_gettime() {
    // Use sentinel values to confirm the syscall overwrites the struct.
    let mut ts = hadron_core::syscall::Timespec { tv_sec: u64::MAX, tv_nsec: u64::MAX };
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") hadron_core::syscall::SYS_CLOCK_GETTIME,
            in("rdi") hadron_core::syscall::CLOCK_MONOTONIC,
            in("rsi") &mut ts as *mut _ as usize,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
        );
    }
    assert_eq!(result, 0, "clock_gettime should succeed");
    // Syscall must have overwritten the sentinel values.
    assert_ne!(ts.tv_nsec, u64::MAX, "tv_nsec should be overwritten");
    // tv_nsec must always be in [0, 999_999_999].
    // Time may be 0 if HPET is not initialized (test harness skips ACPI init).
    assert!(ts.tv_nsec < 1_000_000_000, "tv_nsec must be < 1 billion");
}

#[test_case]
fn test_syscall_clock_gettime_invalid_clock() {
    let mut ts = hadron_core::syscall::Timespec { tv_sec: 0, tv_nsec: 0 };
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") hadron_core::syscall::SYS_CLOCK_GETTIME,
            in("rdi") 99usize, // invalid clock ID
            in("rsi") &mut ts as *mut _ as usize,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
        );
    }
    assert_eq!(
        result,
        -(hadron_core::syscall::EINVAL),
        "invalid clock should return -EINVAL"
    );
}
