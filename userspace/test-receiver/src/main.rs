//! Test receiver process — spawned by userboot during Phase 2c verification.
//!
//! Receives a message + handle via channel_recv_fd on the inherited channel
//! endpoint (handle 1), prints the message, and exits with code 0.

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
    core::arch::naked_asm!("call {main}", "ud2", main = sym main,);
}

/// Test receiver main logic.
fn main() -> ! {
    debug_log("test-receiver: started, waiting for message on handle 1\n");

    // Receive a message with a handle transfer on the inherited channel (fd=1).
    let mut buf = [0u8; 256];
    let mut received_fd: usize = 0;

    let ret = wrappers::sys_channel_recv_fd(
        1, // inherited channel endpoint
        buf.as_mut_ptr() as usize,
        buf.len(),
        &mut received_fd as *mut usize as usize,
    );

    if ret < 0 {
        debug_log("test-receiver: FAIL: channel_recv_fd failed\n");
        wrappers::sys_task_exit(1);
    }

    let msg_len = ret as usize;
    debug_log("test-receiver: received message: ");
    if msg_len <= buf.len() {
        wrappers::sys_debug_log(buf.as_ptr() as usize, msg_len);
    }
    debug_log("\n");

    // We received a handle — verify it's non-zero.
    if received_fd == 0 {
        debug_log("test-receiver: FAIL: no handle received\n");
        wrappers::sys_task_exit(2);
    }

    // Map the received VMO and verify its contents.
    let vaddr = wrappers::sys_mem_map_shared(received_fd, 4096, hadron_syscall::PROT_READ);
    if vaddr < 0 {
        debug_log("test-receiver: FAIL: mem_map_shared failed\n");
        wrappers::sys_task_exit(3);
    }

    // Read and verify the payload written by userboot.
    let expected = b"Phase2-VMO-OK";
    // SAFETY: vaddr is a valid mapped page from the VMO.
    let data = unsafe { core::slice::from_raw_parts(vaddr as *const u8, expected.len()) };

    if data != expected.as_slice() {
        debug_log("test-receiver: FAIL: VMO data mismatch\n");
        wrappers::sys_task_exit(4);
    }

    debug_log("test-receiver: VMO transfer + mapping verified\n");

    // Exit with code 0 to signal success to the parent.
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
