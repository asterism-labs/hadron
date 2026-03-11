//! Test child process — spawned by userboot during Phase 2b verification.
//!
//! Prints a message via debug_log and exits with code 42.

#![no_std]
#![no_main]

use hadron_syscall::wrappers;

/// Write a string to the kernel debug log (serial).
fn debug_log(msg: &str) {
    wrappers::sys_debug_log(msg.as_ptr() as usize, msg.len());
}

/// Raw entry point — calls [`main`] with correct stack alignment.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "call {main}",
        "ud2",
        main = sym main,
    );
}

/// Test child main logic.
fn main() -> ! {
    debug_log("test-child: Hello from child process!\n");
    wrappers::sys_task_exit(42);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // SAFETY: SYS_TASK_EXIT does not return.
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 0x00usize, // SYS_TASK_EXIT
            in("rdi") 99usize,
            options(noreturn, nostack),
        );
    }
}
