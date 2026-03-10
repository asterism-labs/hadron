//! Userboot — the first userspace process.
//!
//! Prints "Hello from userspace" via the `SYS_DEBUG_LOG` syscall, then
//! exits via `SYS_TASK_EXIT`. No runtime dependencies beyond the kernel's
//! syscall interface.

#![no_std]
#![no_main]

/// Syscall number: exit the current task.
const SYS_TASK_EXIT: usize = 0x00;

/// Syscall number: write a byte buffer to the kernel debug log (serial).
const SYS_DEBUG_LOG: usize = 0xF1;

/// Issue a two-argument syscall and return the result.
///
/// # Safety
///
/// The syscall number and arguments must be valid for the kernel's ABI.
#[inline]
unsafe fn syscall2(nr: usize, a0: usize, a1: usize) -> isize {
    let ret: isize;
    // SAFETY: Caller guarantees valid syscall number and arguments.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            // Clobbered by SYSCALL instruction.
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Issue a one-argument syscall that never returns.
///
/// # Safety
///
/// The syscall number and argument must be valid, and the syscall must not
/// return (e.g. `SYS_TASK_EXIT`).
#[inline]
unsafe fn syscall1_noreturn(nr: usize, a0: usize) -> ! {
    // SAFETY: Caller guarantees the syscall does not return.
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") nr,
            in("rdi") a0,
            options(noreturn, nostack),
        );
    }
}

/// Entry point — linked as `_start` by the default linker script.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let msg = b"Hello from userspace\n";
    // SAFETY: SYS_DEBUG_LOG expects a pointer and length in user address space.
    unsafe {
        syscall2(SYS_DEBUG_LOG, msg.as_ptr() as usize, msg.len());
    }
    // SAFETY: SYS_TASK_EXIT terminates the current task and never returns.
    unsafe {
        syscall1_noreturn(SYS_TASK_EXIT, 0);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // SAFETY: SYS_TASK_EXIT terminates the current task and never returns.
    unsafe { syscall1_noreturn(SYS_TASK_EXIT, 1) }
}
