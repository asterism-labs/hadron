//! Userboot — the first userspace process.
//!
//! Phase 2 verification: runs Phase 2a (channel IPC), 2b (spawn/wait),
//! 2c (VMO transfer), and 2d (port aggregation) tests sequentially.

#![no_std]
#![no_main]

use hadron_syscall::types::{FdMapEntry, SpawnInfo};
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

    // ── Phase 2c VMO transfer test ──────────────────────────────────
    test_phase_2c();

    // ── Phase 2d port aggregation test ──────────────────────────────
    test_phase_2d();

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

/// Phase 2c: VMO creation, shared mapping, data transfer over channel.
fn test_phase_2c() {
    debug_log("userboot: Phase 2c VMO transfer test\n");

    // 1. Create a channel pair.
    let mut fds: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(fds.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create failed\n");
        wrappers::sys_task_exit(20);
    }
    let parent_end = fds[0];
    let child_end = fds[1];

    // 2. Spawn test-receiver with child channel endpoint as handle 1.
    let fd_map = [FdMapEntry {
        child_fd: 1,
        parent_fd: child_end as u32,
    }];
    let child_pid = spawn_with_fds("test-receiver", &fd_map);
    if child_pid < 0 {
        debug_log("FAIL: task_spawn test-receiver failed\n");
        wrappers::sys_task_exit(21);
    }
    wrappers::sys_handle_close(child_end);

    // 3. Create a shared VMO and write test data into it.
    let vmo_fd = wrappers::sys_mem_create_shared(4096);
    if vmo_fd < 0 {
        debug_log("FAIL: mem_create_shared failed\n");
        wrappers::sys_task_exit(22);
    }
    let vmo_fd = vmo_fd as usize;

    let vaddr = wrappers::sys_mem_map_shared(
        vmo_fd,
        4096,
        hadron_syscall::PROT_READ | hadron_syscall::PROT_WRITE,
    );
    if vaddr < 0 {
        debug_log("FAIL: mem_map_shared failed\n");
        wrappers::sys_task_exit(23);
    }

    // Write test payload into the VMO via mapped address.
    let payload = b"Phase2-VMO-OK";
    // SAFETY: vaddr is a valid user-mapped page we just allocated.
    unsafe {
        core::ptr::copy_nonoverlapping(payload.as_ptr(), vaddr as *mut u8, payload.len());
    }

    // 4. Send message + VMO handle to child via channel_send_fd.
    let msg = b"Phase2c-VMO";
    let ret = wrappers::sys_channel_send_fd(parent_end, vmo_fd, msg.as_ptr() as usize, msg.len());
    if ret < 0 {
        debug_log("FAIL: channel_send_fd failed\n");
        wrappers::sys_task_exit(24);
    }

    // 5. Wait for child to exit successfully.
    let mut status: usize = 0;
    let ret = wrappers::sys_task_wait(child_pid as usize, &mut status as *mut usize as usize, 0);
    if ret < 0 {
        debug_log("FAIL: task_wait test-receiver failed\n");
        wrappers::sys_task_exit(25);
    }
    if status != 0 {
        debug_log("FAIL: test-receiver exit code != 0\n");
        wrappers::sys_task_exit(26);
    }

    wrappers::sys_handle_close(parent_end);
    debug_log("userboot: Phase 2c PASSED\n");
}

/// Phase 2d: port-based async signal aggregation.
fn test_phase_2d() {
    debug_log("userboot: Phase 2d port aggregation test\n");

    // 1. Create a port for signal aggregation.
    let port_fd = wrappers::sys_port_create();
    if port_fd < 0 {
        debug_log("FAIL: port_create failed\n");
        wrappers::sys_task_exit(30);
    }
    let port_fd = port_fd as usize;

    // 2. Create two channel pairs.
    let mut fds0: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(fds0.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create 0 failed\n");
        wrappers::sys_task_exit(31);
    }
    let ch_a0 = fds0[0];
    let ch_b0 = fds0[1];

    let mut fds1: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(fds1.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create 1 failed\n");
        wrappers::sys_task_exit(32);
    }
    let ch_a1 = fds1[0];
    let ch_b1 = fds1[1];

    // 3. Register async waits on both receive endpoints.
    //    READABLE = SIGNAL_0 = 0x1
    let readable: usize = 0x1;
    let ret = wrappers::sys_object_wait_async(ch_b0, port_fd, 0, readable);
    if ret < 0 {
        debug_log("FAIL: object_wait_async ch_b0 failed\n");
        wrappers::sys_task_exit(33);
    }
    let ret = wrappers::sys_object_wait_async(ch_b1, port_fd, 1, readable);
    if ret < 0 {
        debug_log("FAIL: object_wait_async ch_b1 failed\n");
        wrappers::sys_task_exit(34);
    }

    // 4. Send on ch_a0 → makes ch_b0 READABLE → observer fires → packet on port.
    let msg0 = b"ping0";
    let ret = wrappers::sys_channel_send(ch_a0, msg0.as_ptr() as usize, msg0.len());
    if ret < 0 {
        debug_log("FAIL: channel_send on ch_a0 failed\n");
        wrappers::sys_task_exit(35);
    }

    // 5. Wait on port — expect packet with key=0.
    let mut packet = hadron_syscall::types::UserPortPacket {
        key: u64::MAX,
        signals: 0,
        koid: 0,
        packet_type: 0,
    };
    let ret = wrappers::sys_port_wait(port_fd, &mut packet as *mut _ as usize);
    if ret < 0 {
        debug_log("FAIL: port_wait (first) failed\n");
        wrappers::sys_task_exit(36);
    }
    if packet.key != 0 {
        debug_log("FAIL: expected packet key=0\n");
        wrappers::sys_task_exit(37);
    }

    // 6. Send on ch_a1 → makes ch_b1 READABLE → observer fires → packet on port.
    let msg1 = b"ping1";
    let ret = wrappers::sys_channel_send(ch_a1, msg1.as_ptr() as usize, msg1.len());
    if ret < 0 {
        debug_log("FAIL: channel_send on ch_a1 failed\n");
        wrappers::sys_task_exit(38);
    }

    // 7. Wait on port — expect packet with key=1.
    packet.key = u64::MAX;
    let ret = wrappers::sys_port_wait(port_fd, &mut packet as *mut _ as usize);
    if ret < 0 {
        debug_log("FAIL: port_wait (second) failed\n");
        wrappers::sys_task_exit(39);
    }
    if packet.key != 1 {
        debug_log("FAIL: expected packet key=1\n");
        wrappers::sys_task_exit(40);
    }

    // 8. Clean up.
    wrappers::sys_handle_close(ch_a0);
    wrappers::sys_handle_close(ch_b0);
    wrappers::sys_handle_close(ch_a1);
    wrappers::sys_handle_close(ch_b1);
    wrappers::sys_handle_close(port_fd);

    debug_log("userboot: Phase 2d PASSED\n");
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
