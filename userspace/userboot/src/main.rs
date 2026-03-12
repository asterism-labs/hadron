//! Userboot — the first userspace process.
//!
//! Bootstraps the system: optionally spawns the test harness, then sets up
//! the VFS by spawning ramfs at `/` and devmgr at `/dev`.

#![no_std]
#![no_main]

use hadron_syscall::types::{DirEntryInfo, FdMapEntry, SpawnInfo};
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

/// Userboot main: run test harness (if present), then set up VFS.
fn main() -> ! {
    debug_log("userboot: starting\n");

    // Spawn test-harness if present in initrd (for integration tests).
    let harness_pid = spawn_simple("test-harness");
    if harness_pid >= 0 {
        debug_log("userboot: test-harness spawned, waiting\n");
        let mut status: usize = 0;
        let ret =
            wrappers::sys_task_wait(harness_pid as usize, &mut status as *mut usize as usize, 0);
        if ret < 0 || status != 0 {
            debug_log("userboot: test-harness FAILED\n");
            wrappers::sys_task_exit(1);
        }
        debug_log("userboot: test-harness PASSED\n");
    }

    // Set up VFS: mount ramfs at /, devmgr at /dev.
    setup_vfs();

    debug_log("system ready\n");
    wrappers::sys_task_exit(0);
}

// ── VFS setup ────────────────────────────────────────────────────────

/// Initrd data channel passed by the kernel as handle 3.
const INITRD_HANDLE: usize = 3;

/// Set up the VFS: spawn ramfs at `/`, devmgr at `/dev`, verify readdir.
fn setup_vfs() {
    // 1. Create a channel pair for the ramfs mount point.
    let mut ramfs_fds: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(ramfs_fds.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create for ramfs mount\n");
        wrappers::sys_task_exit(50);
    }
    let ramfs_userboot_end = ramfs_fds[0];
    let ramfs_server_end = ramfs_fds[1];

    // 2. Spawn ramfs with mount channel (handle 0) and initrd channel (handle 3).
    let fd_map = [
        FdMapEntry {
            child_fd: 0,
            parent_fd: ramfs_server_end as u32,
        },
        FdMapEntry {
            child_fd: 3,
            parent_fd: INITRD_HANDLE as u32,
        },
    ];
    let ramfs_pid = spawn_with_fds("ramfs", &fd_map);
    if ramfs_pid < 0 {
        debug_log("FAIL: spawn ramfs\n");
        wrappers::sys_task_exit(51);
    }
    wrappers::sys_handle_close(ramfs_server_end);

    debug_log("userboot: ramfs spawned, mounting /\n");

    // 3. Mount ramfs at "/".
    let prefix = "/";
    let ret = wrappers::sys_vfs_mount(prefix.as_ptr() as usize, prefix.len(), ramfs_userboot_end);
    if ret < 0 {
        debug_log("FAIL: vfs_mount / failed\n");
        wrappers::sys_task_exit(52);
    }

    debug_log("userboot: / mounted\n");

    // 4. Create a channel pair for devmgr mount point.
    let mut devmgr_fds: [usize; 2] = [0; 2];
    let ret = wrappers::sys_channel_create(devmgr_fds.as_mut_ptr() as usize);
    if ret < 0 {
        debug_log("FAIL: channel_create for devmgr mount\n");
        wrappers::sys_task_exit(53);
    }
    let devmgr_userboot_end = devmgr_fds[0];
    let devmgr_server_end = devmgr_fds[1];

    // 5. Spawn devmgr with handle 0 = mount channel.
    let fd_map = [FdMapEntry {
        child_fd: 0,
        parent_fd: devmgr_server_end as u32,
    }];
    let devmgr_pid = spawn_with_fds("devmgr", &fd_map);
    if devmgr_pid < 0 {
        debug_log("FAIL: spawn devmgr\n");
        wrappers::sys_task_exit(54);
    }
    wrappers::sys_handle_close(devmgr_server_end);

    debug_log("userboot: devmgr spawned, mounting /dev\n");

    // 6. Mount devmgr at "/dev".
    let prefix = "/dev";
    let ret = wrappers::sys_vfs_mount(prefix.as_ptr() as usize, prefix.len(), devmgr_userboot_end);
    if ret < 0 {
        debug_log("FAIL: vfs_mount /dev failed\n");
        wrappers::sys_task_exit(55);
    }

    debug_log("userboot: /dev mounted\n");

    // 7. Verify by opening "/" and reading directory entries.
    let path = "/";
    let vnode_fd = wrappers::sys_vnode_open(path.as_ptr() as usize, path.len(), 0);
    if vnode_fd < 0 {
        debug_log("FAIL: vnode_open / failed\n");
        wrappers::sys_task_exit(56);
    }

    let mut entries_buf = [0u8; 1056];
    let n = wrappers::sys_vnode_readdir(
        vnode_fd as usize,
        entries_buf.as_mut_ptr() as usize,
        entries_buf.len(),
    );

    if n < 0 {
        debug_log("FAIL: vnode_readdir / failed\n");
        wrappers::sys_task_exit(57);
    }

    let n = n as usize;
    let entry_size = core::mem::size_of::<DirEntryInfo>();
    let entry_count = n / entry_size;

    debug_log("ls /:");
    let mut i = 0;
    while i < entry_count {
        let offset = i * entry_size;
        // SAFETY: We verified the buffer has enough data.
        let entry: &DirEntryInfo =
            unsafe { &*entries_buf[offset..].as_ptr().cast::<DirEntryInfo>() };
        let name = entry.name_str();
        debug_log(" ");
        debug_log(name);
        i += 1;
    }
    debug_log("\n");

    wrappers::sys_handle_close(vnode_fd as usize);

    // Don't wait for ramfs/devmgr (they run forever as servers).
    wrappers::sys_handle_close(ramfs_userboot_end);
    wrappers::sys_handle_close(devmgr_userboot_end);
    let _ = (ramfs_pid, devmgr_pid);
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
