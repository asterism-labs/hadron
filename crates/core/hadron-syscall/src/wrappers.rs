//! Inline-assembly syscall stubs for userspace.
//!
//! Each `sys_*` function issues a `syscall` instruction with the appropriate
//! number and arguments. These are only compiled when `target_os = "hadron"`.

use crate::numbers::*;

// ── Raw syscall primitives ───────────────────────────────────────────

#[inline]
fn syscall0(nr: usize) -> isize {
    let ret: isize;
    // SAFETY: Syscall ABI — nr in RAX, result in RAX.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
fn syscall1(nr: usize, a0: usize) -> isize {
    let ret: isize;
    // SAFETY: Syscall ABI — nr in RAX, a0 in RDI.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
fn syscall1_noreturn(nr: usize, a0: usize) -> ! {
    // SAFETY: Syscall ABI — the syscall does not return.
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") nr,
            in("rdi") a0,
            options(noreturn, nostack),
        );
    }
}

#[inline]
fn syscall2(nr: usize, a0: usize, a1: usize) -> isize {
    let ret: isize;
    // SAFETY: Syscall ABI — nr in RAX, a0 in RDI, a1 in RSI.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    // SAFETY: Syscall ABI — nr in RAX, a0 in RDI, a1 in RSI, a2 in RDX.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
fn syscall4(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    // SAFETY: Syscall ABI — nr in RAX, a0 in RDI, a1 in RSI, a2 in RDX,
    // a3 in R10 (not RCX, which is clobbered by SYSCALL).
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

#[inline]
fn syscall5(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
    let ret: isize;
    // SAFETY: Syscall ABI — a4 in R8.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            in("r8") a4,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

// ── Task ─────────────────────────────────────────────────────────────

/// Terminate the current process.
pub fn sys_task_exit(status: usize) -> ! {
    syscall1_noreturn(SYS_TASK_EXIT, status);
}

/// Spawn a new process. `info_ptr` points to a [`SpawnInfo`](crate::SpawnInfo).
pub fn sys_task_spawn(info_ptr: usize, info_len: usize) -> isize {
    syscall2(SYS_TASK_SPAWN, info_ptr, info_len)
}

/// Wait for a child process to exit.
pub fn sys_task_wait(pid: usize, status_ptr: usize, flags: usize) -> isize {
    syscall3(SYS_TASK_WAIT, pid, status_ptr, flags)
}

/// Send a signal to a process.
pub fn sys_task_kill(pid: usize, signum: usize) -> isize {
    syscall2(SYS_TASK_KILL, pid, signum)
}

/// Query current process information (returns PID/koid).
pub fn sys_task_info() -> isize {
    syscall0(SYS_TASK_INFO)
}

/// Register a signal handler.
pub fn sys_task_sigaction(
    signum: usize,
    handler: usize,
    flags: usize,
    old_handler_ptr: usize,
) -> isize {
    syscall4(SYS_TASK_SIGACTION, signum, handler, flags, old_handler_ptr)
}

/// Set process group ID.
pub fn sys_task_setpgid(pid: usize, pgid: usize) -> isize {
    syscall2(SYS_TASK_SETPGID, pid, pgid)
}

/// Get process group ID.
pub fn sys_task_getpgid(pid: usize) -> isize {
    syscall1(SYS_TASK_GETPGID, pid)
}

// ── Handle ───────────────────────────────────────────────────────────

/// Close a handle.
pub fn sys_handle_close(fd: usize) -> isize {
    syscall1(SYS_HANDLE_CLOSE, fd)
}

/// Duplicate a handle to a specific slot.
pub fn sys_handle_dup(old_fd: usize, new_fd: usize) -> isize {
    syscall2(SYS_HANDLE_DUP, old_fd, new_fd)
}

/// Duplicate a handle to the lowest available slot.
pub fn sys_handle_dup_lowest(fd: usize) -> isize {
    syscall1(SYS_HANDLE_DUP_LOWEST, fd)
}

/// Create a pipe (socket pair). `fds_ptr` points to `[usize; 2]`.
pub fn sys_handle_pipe(fds_ptr: usize) -> isize {
    syscall1(SYS_HANDLE_PIPE, fds_ptr)
}

/// Set terminal foreground process group.
pub fn sys_handle_tcsetpgrp(fd: usize, pgid: usize) -> isize {
    syscall2(SYS_HANDLE_TCSETPGRP, fd, pgid)
}

/// Get terminal foreground process group.
pub fn sys_handle_tcgetpgrp(fd: usize) -> isize {
    syscall1(SYS_HANDLE_TCGETPGRP, fd)
}

// ── Channel ──────────────────────────────────────────────────────────

/// Create a channel pair. `fds_ptr` points to `[usize; 2]`.
pub fn sys_channel_create(fds_ptr: usize) -> isize {
    syscall1(SYS_CHANNEL_CREATE, fds_ptr)
}

/// Send a message on a channel.
pub fn sys_channel_send(fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall3(SYS_CHANNEL_SEND, fd, buf_ptr, len)
}

/// Receive a message from a channel.
pub fn sys_channel_recv(fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall3(SYS_CHANNEL_RECV, fd, buf_ptr, len)
}

/// Accept a connection on a listener.
pub fn sys_channel_accept(fd: usize) -> isize {
    syscall1(SYS_CHANNEL_ACCEPT, fd)
}

/// Send a message with an attached handle.
pub fn sys_channel_send_fd(ch_fd: usize, fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall4(SYS_CHANNEL_SEND_FD, ch_fd, fd, buf_ptr, len)
}

/// Receive a message with an attached handle.
pub fn sys_channel_recv_fd(ch_fd: usize, buf_ptr: usize, len: usize, fd_out_ptr: usize) -> isize {
    syscall4(SYS_CHANNEL_RECV_FD, ch_fd, buf_ptr, len, fd_out_ptr)
}

// ── EventPair ────────────────────────────────────────────────────

/// Create an event pair. `fds_ptr` points to `[usize; 2]`.
pub fn sys_event_pair_create(fds_ptr: usize) -> isize {
    syscall1(SYS_EVENT_PAIR_CREATE, fds_ptr)
}

/// Signal the peer endpoint of an event pair.
pub fn sys_event_pair_signal_peer(fd: usize, set_mask: usize, clear_mask: usize) -> isize {
    syscall3(SYS_EVENT_PAIR_SIGNAL_PEER, fd, set_mask, clear_mask)
}

// ── FIFO ─────────────────────────────────────────────────────────

/// Create a FIFO pair. `fds_ptr` points to `[usize; 2]`.
pub fn sys_fifo_create(fds_ptr: usize, elem_count: usize, elem_size: usize) -> isize {
    syscall3(SYS_FIFO_CREATE, fds_ptr, elem_count, elem_size)
}

/// Write elements to a FIFO.
pub fn sys_fifo_write(fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall3(SYS_FIFO_WRITE, fd, buf_ptr, len)
}

/// Read elements from a FIFO.
pub fn sys_fifo_read(fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall3(SYS_FIFO_READ, fd, buf_ptr, len)
}

// ── Port ─────────────────────────────────────────────────────────

/// Create a port (async event aggregator).
pub fn sys_port_create() -> isize {
    syscall0(SYS_PORT_CREATE)
}

/// Wait for a packet on a port. `packet_ptr` points to a `UserPortPacket`.
pub fn sys_port_wait(fd: usize, packet_ptr: usize) -> isize {
    syscall2(SYS_PORT_WAIT, fd, packet_ptr)
}

/// Queue a user packet on a port.
pub fn sys_port_queue(fd: usize, key: usize, signals: usize) -> isize {
    syscall3(SYS_PORT_QUEUE, fd, key, signals)
}

/// Register an async wait: when `object_fd`'s signals match `signals`,
/// deliver a packet with `key` to `port_fd`.
pub fn sys_object_wait_async(
    object_fd: usize,
    port_fd: usize,
    key: usize,
    signals: usize,
) -> isize {
    syscall4(SYS_OBJECT_WAIT_ASYNC, object_fd, port_fd, key, signals)
}

// ── Timer ────────────────────────────────────────────────────────

/// Create a timer object.
pub fn sys_timer_create() -> isize {
    syscall0(SYS_TIMER_CREATE)
}

/// Set a timer deadline (nanoseconds since boot).
pub fn sys_timer_set(fd: usize, deadline_ns: usize, slack_ns: usize) -> isize {
    syscall3(SYS_TIMER_SET, fd, deadline_ns, slack_ns)
}

/// Cancel a pending timer.
pub fn sys_timer_cancel(fd: usize) -> isize {
    syscall1(SYS_TIMER_CANCEL, fd)
}

// ── Event ────────────────────────────────────────────────────────

/// Create an event object.
pub fn sys_event_create() -> isize {
    syscall0(SYS_EVENT_CREATE)
}

/// Signal an event object (set/clear signal bits).
pub fn sys_event_signal(fd: usize, set_mask: usize, clear_mask: usize) -> isize {
    syscall3(SYS_EVENT_SIGNAL, fd, set_mask, clear_mask)
}

/// Wait for signals on a single object.
pub fn sys_event_wait(fd: usize, signals: usize) -> isize {
    syscall2(SYS_EVENT_WAIT, fd, signals)
}

// ── Vnode ────────────────────────────────────────────────────────────

/// Open a file.
pub fn sys_vnode_open(path_ptr: usize, path_len: usize, flags: usize) -> isize {
    syscall3(SYS_VNODE_OPEN, path_ptr, path_len, flags)
}

/// Read from a file descriptor.
pub fn sys_vnode_read(fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall3(SYS_VNODE_READ, fd, buf_ptr, len)
}

/// Write to a file descriptor.
pub fn sys_vnode_write(fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall3(SYS_VNODE_WRITE, fd, buf_ptr, len)
}

/// Get file status.
pub fn sys_vnode_stat(fd: usize, buf_ptr: usize) -> isize {
    syscall2(SYS_VNODE_STAT, fd, buf_ptr)
}

/// Read directory entries.
pub fn sys_vnode_readdir(fd: usize, buf_ptr: usize, len: usize) -> isize {
    syscall3(SYS_VNODE_READDIR, fd, buf_ptr, len)
}

/// Seek within a file.
pub fn sys_vnode_seek(fd: usize, offset: usize, whence: usize) -> isize {
    syscall3(SYS_VNODE_SEEK, fd, offset, whence)
}

// ── Memory ───────────────────────────────────────────────────────────

/// Map memory into the address space.
pub fn sys_mem_map(addr_hint: usize, len: usize, prot: usize, flags: usize, fd: usize) -> isize {
    syscall5(SYS_MEM_MAP, addr_hint, len, prot, flags, fd)
}

/// Unmap a memory region.
pub fn sys_mem_unmap(addr: usize, len: usize) -> isize {
    syscall2(SYS_MEM_UNMAP, addr, len)
}

/// Adjust the program break.
pub fn sys_mem_brk(addr: usize) -> isize {
    syscall1(SYS_MEM_BRK, addr)
}

/// Create a shared memory object.
pub fn sys_mem_create_shared(size: usize) -> isize {
    syscall1(SYS_MEM_CREATE_SHARED, size)
}

/// Map a shared memory object.
pub fn sys_mem_map_shared(fd: usize, size: usize, prot: usize) -> isize {
    syscall3(SYS_MEM_MAP_SHARED, fd, size, prot)
}

// ── Event ────────────────────────────────────────────────────────────

/// Poll multiple file descriptors.
pub fn sys_event_wait_many(fds_ptr: usize, nfds: usize, timeout_ms: usize) -> isize {
    syscall3(SYS_EVENT_WAIT_MANY, fds_ptr, nfds, timeout_ms)
}

/// Get the current time.
pub fn sys_clock_gettime(clockid: u32, tp_ptr: usize) -> isize {
    syscall2(SYS_CLOCK_GETTIME, clockid as usize, tp_ptr)
}

/// Sleep for a specified duration.
pub fn sys_clock_nanosleep(clockid: u32, flags: usize, req_ptr: usize, rem_ptr: usize) -> isize {
    syscall4(
        SYS_CLOCK_NANOSLEEP,
        clockid as usize,
        flags,
        req_ptr,
        rem_ptr,
    )
}

/// Futex operations.
pub fn sys_futex(addr: usize, op: usize, val: usize, timeout: usize) -> isize {
    syscall4(SYS_FUTEX, addr, op, val, timeout)
}

// ── VFS ─────────────────────────────────────────────────────────────

/// Mount a filesystem at a path prefix.
pub fn sys_vfs_mount(prefix_ptr: usize, prefix_len: usize, channel_fd: usize) -> isize {
    syscall3(SYS_VFS_MOUNT, prefix_ptr, prefix_len, channel_fd)
}

/// Unmount a filesystem at a path prefix.
pub fn sys_vfs_unmount(prefix_ptr: usize, prefix_len: usize) -> isize {
    syscall2(SYS_VFS_UNMOUNT, prefix_ptr, prefix_len)
}

// ── System ───────────────────────────────────────────────────────────

/// Query system information.
pub fn sys_query(query_type: usize, sub_id: usize, buf_ptr: usize, buf_len: usize) -> isize {
    syscall4(SYS_QUERY, query_type, sub_id, buf_ptr, buf_len)
}

/// Write to the kernel debug log.
pub fn sys_debug_log(buf_ptr: usize, len: usize) -> isize {
    syscall2(SYS_DEBUG_LOG, buf_ptr, len)
}
