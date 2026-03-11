//! Userboot — the first userspace process.
//!
//! Phase 2b verification: creates a channel pair (IPC test from 2a),
//! then spawns a child process, waits for it to exit, and verifies
//! the exit code.

#![no_std]
#![no_main]

use hadron_syscall::wrappers;

/// Write a string to the kernel debug log (serial).
fn debug_log(msg: &str) {
    wrappers::sys_debug_log(msg.as_ptr() as usize, msg.len());
}

/// Raw entry point — calls [`main`] with correct stack alignment.
///
/// The kernel enters `_start` via `iretq` with RSP 16-byte aligned.
/// The `call` instruction pushes 8 bytes (return address), making
/// RSP ≡ 8 (mod 16) at `main` entry — exactly what the SysV ABI expects.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "call {main}",
        "ud2",
        main = sym main,
    );
}

/// Userboot main logic — Phase 2b verification.
fn main() -> ! {
    debug_log("userboot: starting Phase 2b verification\n");

    // ── Phase 2a IPC test (channel self-send/recv) ───────────────────
    debug_log("userboot: Phase 2a IPC test\n");

    let mut fds: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(fds.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create failed\n");
        wrappers::sys_task_exit(1);
    }
    let fd_a = fds[0];
    let fd_b = fds[1];

    let msg = b"hello from channel";
    let ret = wrappers::sys_channel_send(fd_a, msg.as_ptr() as usize, msg.len());
    if ret < 0 {
        debug_log("FAIL: channel_send failed\n");
        wrappers::sys_task_exit(2);
    }

    let mut buf = [0u8; 64];
    let ret = wrappers::sys_channel_recv(fd_b, buf.as_mut_ptr() as usize, buf.len());
    if ret < 0 {
        debug_log("FAIL: channel_recv failed\n");
        wrappers::sys_task_exit(3);
    }

    let received_len = ret as usize;
    if received_len != msg.len() || &buf[..received_len] != msg.as_slice() {
        debug_log("FAIL: received data mismatch\n");
        wrappers::sys_task_exit(4);
    }

    // Close endpoints.
    wrappers::sys_handle_close(fd_a);
    wrappers::sys_handle_close(fd_b);

    debug_log("userboot: Phase 2a IPC test passed\n");

    // ── Phase 2b spawn test ──────────────────────────────────────────
    debug_log("userboot: spawning test-child\n");

    let path = b"test-child";
    let child_pid = wrappers::sys_task_spawn(path.as_ptr() as usize, path.len());
    if child_pid < 0 {
        debug_log("FAIL: task_spawn failed\n");
        wrappers::sys_task_exit(10);
    }

    debug_log("userboot: waiting for child\n");

    let mut status: usize = 0;
    let ret = wrappers::sys_task_wait(child_pid as usize, &mut status as *mut usize as usize, 0);
    if ret < 0 {
        debug_log("FAIL: task_wait failed\n");
        wrappers::sys_task_exit(11);
    }

    // Verify child exited with code 42.
    if status != 42 {
        debug_log("FAIL: child exit code mismatch\n");
        wrappers::sys_task_exit(12);
    }

    debug_log("Phase 2b verification passed!\n");
    wrappers::sys_task_exit(0);
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
