//! Time syscall handlers: clock_gettime, clock_nanosleep.

use crate::syscall::userptr::{UserPtr, is_kernel_caller};
use crate::syscall::{CLOCK_MONOTONIC, CLOCK_REALTIME, EFAULT, EINVAL, Timespec};

/// `sys_clock_gettime` — returns monotonic or real-time clock value.
///
/// Supports `CLOCK_MONOTONIC` (boot-relative via HPET) and `CLOCK_REALTIME`
/// (Unix epoch via CMOS RTC + HPET).
pub(super) fn sys_clock_gettime(clock_id: usize, tp: usize) -> isize {
    let nanos = match clock_id {
        CLOCK_MONOTONIC => crate::time::Time::boot_nanos(),
        CLOCK_REALTIME => crate::time::Time::realtime_nanos(),
        _ => return -EINVAL,
    };

    let ts = Timespec {
        tv_sec: nanos / 1_000_000_000,
        tv_nsec: nanos % 1_000_000_000,
    };

    if is_kernel_caller(tp) {
        // Kernel-mode test: tp is a kernel address, write directly.
        // SAFETY: Kernel-mode callers pass a valid stack-local `&mut Timespec`.
        // The address is in the upper half (verified by `is_kernel_caller`).
        unsafe { core::ptr::write(tp as *mut Timespec, ts) };
    } else {
        let user_ptr = match UserPtr::<Timespec>::new(tp) {
            Ok(p) => p,
            Err(_) => return -EFAULT,
        };
        // SAFETY: UserPtr validated that the address is in user space,
        // properly aligned, and does not overflow. The caller is responsible
        // for ensuring the page is mapped and writable.
        unsafe { core::ptr::write(user_ptr.addr() as *mut Timespec, ts) };
    }

    0
}

/// `sys_clock_nanosleep` — high-resolution sleep.
///
/// Suspends the calling task for the duration specified in the `Timespec` at
/// `req_ptr`. Only `CLOCK_MONOTONIC` is supported. `flags` is reserved and
/// must be 0.
///
/// This is a blocking syscall — it triggers a TRAP_SLEEP longjmp back to
/// `process_task`, which awaits the sleep future.
pub(super) fn sys_clock_nanosleep(
    clock_id: usize,
    _flags: usize,
    req_ptr: usize,
    _rem_ptr: usize,
) -> isize {
    if clock_id != CLOCK_MONOTONIC {
        return -EINVAL;
    }

    // Read the requested duration from user memory.
    let ts = if is_kernel_caller(req_ptr) {
        // SAFETY: Kernel-mode callers pass a valid stack-local `&Timespec`.
        unsafe { core::ptr::read(req_ptr as *const Timespec) }
    } else {
        let user_ptr = match UserPtr::<Timespec>::new(req_ptr) {
            Ok(p) => p,
            Err(_) => return -EFAULT,
        };
        // SAFETY: UserPtr validated the address.
        unsafe { core::ptr::read(user_ptr.addr() as *const Timespec) }
    };

    // Convert to milliseconds (timer ticks at 1kHz).
    let ms = ts
        .tv_sec
        .saturating_mul(1000)
        .saturating_add(ts.tv_nsec / 1_000_000);

    if ms == 0 {
        return 0; // Zero sleep, return immediately.
    }

    // Trigger TRAP_SLEEP to block in process_task.
    trap_sleep(ms)
}

/// Trigger a TRAP_SLEEP longjmp back to `process_task`.
///
/// Sets the sleep duration, restores kernel CR3 and GS bases, then
/// calls `restore_kernel_context` — never returns.
fn trap_sleep(ms: u64) -> ! {
    use crate::arch::x86_64::registers::control::Cr3;
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
    use crate::arch::x86_64::userspace::restore_kernel_context;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();

    // SAFETY: Restoring kernel CR3 and GS bases is the standard pattern
    // for returning from userspace context to kernel context.
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    crate::proc::SleepState::set_ms(ms);
    crate::proc::TrapContext::set_trap_reason(crate::proc::TrapReason::Sleep);

    let saved_rsp = crate::proc::TrapContext::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}
