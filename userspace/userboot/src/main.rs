//! Userboot — the first userspace process.
//!
//! Phase 2a verification: creates a channel pair, sends a message on one
//! endpoint, receives it from the other, and prints success via debug_log.

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

/// Userboot main logic — Phase 2a IPC verification.
fn main() -> ! {
    debug_log("userboot: starting Phase 2a IPC test\n");

    // Step 1: Create a channel pair.
    let mut fds: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(fds.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create failed\n");
        wrappers::sys_task_exit(1);
    }
    let fd_a = fds[0];
    let fd_b = fds[1];

    // Step 2: Send a message on fd_a.
    let msg = b"hello from channel";
    let ret = wrappers::sys_channel_send(fd_a, msg.as_ptr() as usize, msg.len());
    if ret < 0 {
        debug_log("FAIL: channel_send failed\n");
        wrappers::sys_task_exit(2);
    }

    // Step 3: Receive the message on fd_b.
    let mut buf = [0u8; 64];
    let ret = wrappers::sys_channel_recv(fd_b, buf.as_mut_ptr() as usize, buf.len());
    if ret < 0 {
        debug_log("FAIL: channel_recv failed\n");
        wrappers::sys_task_exit(3);
    }

    // Step 4: Verify the received message matches.
    let received_len = ret as usize;
    if received_len != msg.len() {
        debug_log("FAIL: received length mismatch\n");
        wrappers::sys_task_exit(4);
    }

    let received = &buf[..received_len];
    if received != msg.as_slice() {
        debug_log("FAIL: received data mismatch\n");
        wrappers::sys_task_exit(5);
    }

    // Step 5: Test handle close.
    let ret = wrappers::sys_handle_close(fd_a);
    if ret < 0 {
        debug_log("FAIL: handle_close failed\n");
        wrappers::sys_task_exit(6);
    }

    // Receiving from fd_b after peer closed should return EAGAIN (empty) or EPIPE.
    let ret = wrappers::sys_channel_recv(fd_b, buf.as_mut_ptr() as usize, buf.len());
    // After peer close and queue empty, should get -EAGAIN or -EPIPE.
    if ret >= 0 {
        debug_log("FAIL: recv after peer close should fail\n");
        wrappers::sys_task_exit(7);
    }

    debug_log("Phase 2a IPC test passed!\n");
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
