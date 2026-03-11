//! Userboot — the first userspace process.
//!
//! Phase 2 verification: runs Phase 2a (channel IPC), 2b (spawn/wait),
//! and 2c (handle transfer + event poll) tests sequentially.

#![no_std]
#![no_main]

use hadron_syscall::types::{FdMapEntry, PollFd, SpawnInfo};
use hadron_syscall::wrappers;

/// Write a string to the kernel debug log (serial).
fn debug_log(msg: &str) {
    wrappers::sys_debug_log(msg.as_ptr() as usize, msg.len());
}

/// Spawn a process by path with no handle inheritance.
fn spawn_simple(path: &str) -> isize {
    let info = SpawnInfo {
        path_ptr: path.as_ptr() as usize,
        path_len: path.len(),
        argv_ptr: 0,
        argv_count: 0,
        envp_ptr: 0,
        envp_count: 0,
        fd_map_ptr: 0,
        fd_map_count: 0,
        cwd_ptr: 0,
        cwd_len: 0,
    };
    wrappers::sys_task_spawn(
        &info as *const SpawnInfo as usize,
        core::mem::size_of::<SpawnInfo>(),
    )
}

/// Spawn a process with handle inheritance via fd_map.
fn spawn_with_fds(path: &str, fd_map: &[FdMapEntry]) -> isize {
    let info = SpawnInfo {
        path_ptr: path.as_ptr() as usize,
        path_len: path.len(),
        argv_ptr: 0,
        argv_count: 0,
        envp_ptr: 0,
        envp_count: 0,
        fd_map_ptr: if fd_map.is_empty() {
            0
        } else {
            fd_map.as_ptr() as usize
        },
        fd_map_count: fd_map.len(),
        cwd_ptr: 0,
        cwd_len: 0,
    };
    wrappers::sys_task_spawn(
        &info as *const SpawnInfo as usize,
        core::mem::size_of::<SpawnInfo>(),
    )
}

/// Raw entry point — calls [`main`] with correct stack alignment.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _start() -> ! {
    core::arch::naked_asm!("call {main}", "ud2", main = sym main,);
}

/// Userboot main logic — Phase 2 verification.
fn main() -> ! {
    debug_log("userboot: starting Phase 2 verification\n");

    // ── Phase 2a IPC test (channel self-send/recv) ───────────────────
    test_phase_2a();

    // ── Phase 2b spawn test ──────────────────────────────────────────
    test_phase_2b();

    // ── Phase 2c handle transfer + poll test ─────────────────────────
    test_phase_2c();

    debug_log("Phase 2 verification PASSED!\n");
    wrappers::sys_task_exit(0);
}

/// Phase 2a: channel self-send/recv.
fn test_phase_2a() {
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

    wrappers::sys_handle_close(fd_a);
    wrappers::sys_handle_close(fd_b);

    debug_log("userboot: Phase 2a PASSED\n");
}

/// Phase 2b: spawn child, wait for exit code 42.
fn test_phase_2b() {
    debug_log("userboot: Phase 2b spawn test\n");

    let child_pid = spawn_simple("test-child");
    if child_pid < 0 {
        debug_log("FAIL: task_spawn failed\n");
        wrappers::sys_task_exit(10);
    }

    let mut status: usize = 0;
    let ret = wrappers::sys_task_wait(child_pid as usize, &mut status as *mut usize as usize, 0);
    if ret < 0 {
        debug_log("FAIL: task_wait failed\n");
        wrappers::sys_task_exit(11);
    }

    if status != 42 {
        debug_log("FAIL: child exit code mismatch\n");
        wrappers::sys_task_exit(12);
    }

    debug_log("userboot: Phase 2b PASSED\n");
}

/// Phase 2c: handle transfer over channel + event_wait_many polling.
fn test_phase_2c() {
    debug_log("userboot: Phase 2c handle transfer + poll test\n");

    // 1. Create a channel pair.
    let mut fds: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(fds.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create failed\n");
        wrappers::sys_task_exit(20);
    }
    let parent_end = fds[0]; // parent keeps this
    let child_end = fds[1]; // child gets this

    // 2. Spawn test-receiver with the child channel endpoint as handle 1.
    let fd_map = [FdMapEntry {
        child_fd: 1,
        parent_fd: child_end as u32,
    }];
    let child_pid = spawn_with_fds("test-receiver", &fd_map);
    if child_pid < 0 {
        debug_log("FAIL: task_spawn test-receiver failed\n");
        wrappers::sys_task_exit(21);
    }

    // Close our copy of the child's endpoint.
    wrappers::sys_handle_close(child_end);

    // 3. Create a second channel to use as the "transferred handle".
    //    (The plan says VMO, but channels work for handle transfer verification.)
    let mut extra_fds: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(extra_fds.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: extra channel_create failed\n");
        wrappers::sys_task_exit(22);
    }
    let transfer_handle = extra_fds[0];
    // Close the other end — we just want to transfer one handle.
    wrappers::sys_handle_close(extra_fds[1]);

    // 4. Send a message + handle to the child via channel_send_fd.
    let msg = b"Phase2c-IPC";
    let ret = wrappers::sys_channel_send_fd(
        parent_end,
        transfer_handle,
        msg.as_ptr() as usize,
        msg.len(),
    );
    if ret < 0 {
        debug_log("FAIL: channel_send_fd failed\n");
        wrappers::sys_task_exit(23);
    }

    // 5. Poll the parent endpoint with event_wait_many.
    //    After the child receives and exits, we expect PEER_CLOSED (POLLHUP)
    //    on our end since the child's endpoint was closed.
    //    First, wait for the child to exit so we know the peer is closed.
    let mut status: usize = 0;
    let ret = wrappers::sys_task_wait(child_pid as usize, &mut status as *mut usize as usize, 0);
    if ret < 0 {
        debug_log("FAIL: task_wait test-receiver failed\n");
        wrappers::sys_task_exit(24);
    }
    if status != 0 {
        debug_log("FAIL: test-receiver exit code != 0\n");
        wrappers::sys_task_exit(25);
    }

    // Now poll — the peer is closed, so we should see POLLHUP.
    let mut poll_fds = [PollFd {
        fd: parent_end as u32,
        events: hadron_syscall::constants::POLLIN,
        revents: 0,
    }];
    let ret = wrappers::sys_event_wait_many(
        poll_fds.as_mut_ptr() as usize,
        poll_fds.len(),
        0, // non-blocking
    );
    if ret < 0 {
        debug_log("FAIL: event_wait_many failed\n");
        wrappers::sys_task_exit(26);
    }

    // We expect POLLHUP because the peer channel endpoint was closed
    // when the child process exited.
    if poll_fds[0].revents & hadron_syscall::constants::POLLHUP == 0 {
        debug_log("FAIL: expected POLLHUP on closed peer\n");
        wrappers::sys_task_exit(27);
    }

    wrappers::sys_handle_close(parent_end);

    debug_log("userboot: Phase 2c PASSED\n");
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
