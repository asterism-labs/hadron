//! Typed syscall wrappers returning `Result<T, Errno>`.
//!
//! Each function calls the corresponding `hadron_syscall::wrappers::sys_*`
//! function and translates negative return values to `Err(Errno(-ret))`.

use crate::errno::Errno;

/// Convert a raw syscall return (isize) to `Result<usize, Errno>`.
/// Negative values indicate a negated errno.
#[inline]
fn check(ret: isize) -> Result<usize, Errno> {
    if ret < 0 {
        Err(Errno((-ret) as i32))
    } else {
        Ok(ret as usize)
    }
}

/// Convert a raw syscall return to `Result<(), Errno>`.
#[inline]
fn check_unit(ret: isize) -> Result<(), Errno> {
    check(ret).map(|_| ())
}

// ---- Process -----------------------------------------------------------------

pub fn sys_exit(status: usize) -> ! {
    hadron_syscall::wrappers::sys_task_exit(status);
    // The kernel never returns, but the wrapper returns isize.
    // Ensure we never continue.
    loop {}
}

pub fn sys_getpid() -> usize {
    // task_info returns current PID
    hadron_syscall::wrappers::sys_task_info() as usize
}

pub fn sys_getppid() -> usize {
    hadron_syscall::wrappers::sys_task_getppid() as usize
}

pub fn sys_waitpid(pid: usize, status: *mut u64, flags: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_task_wait(
        pid,
        status as usize,
        flags,
    ))
}

pub fn sys_kill(pid: usize, sig: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_task_kill(pid, sig))
}

pub fn sys_getcwd(buf: *mut u8, size: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_task_getcwd(
        buf as usize,
        size,
    ))
}

pub fn sys_chdir(path: &[u8]) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_task_chdir(
        path.as_ptr() as usize,
        path.len(),
    ))
}

pub fn sys_sigaction(
    sig: usize,
    handler: usize,
    flags: usize,
    old_handler: *mut usize,
) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_task_sigaction(
        sig,
        handler,
        flags,
        old_handler as usize,
    ))
}

pub fn sys_sigprocmask(how: usize, set: *const u64, oldset: *mut u64) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_task_sigprocmask(
        how,
        set as usize,
        oldset as usize,
    ))
}

pub fn sys_setpgid(pid: usize, pgid: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_task_setpgid(pid, pgid))
}

pub fn sys_getpgid(pid: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_task_getpgid(pid))
}

pub fn sys_setsid() -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_task_setsid())
}

pub fn sys_execve(info_ptr: *const u8, info_len: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_task_execve(
        info_ptr as usize,
        info_len,
    ))
}

// ---- File descriptors --------------------------------------------------------

pub fn sys_close(fd: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_handle_close(fd))
}

pub fn sys_dup(old_fd: usize, new_fd: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_handle_dup(old_fd, new_fd))
}

pub fn sys_dup_lowest(fd: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_handle_dup_lowest(fd))
}

pub fn sys_pipe(fds: *mut [usize; 2]) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_handle_pipe(fds as usize))
}

pub fn sys_pipe2(fds: *mut [usize; 2], flags: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_handle_pipe2(
        fds as usize,
        flags,
    ))
}

pub fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_handle_fcntl(fd, cmd, arg))
}

pub fn sys_ioctl(fd: usize, cmd: usize, arg: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_handle_ioctl(fd, cmd, arg))
}

// ---- Filesystem --------------------------------------------------------------

pub fn sys_open(path: &[u8], flags: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_vnode_open(
        path.as_ptr() as usize,
        path.len(),
        flags,
    ))
}

pub fn sys_read(fd: usize, buf: &mut [u8]) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_vnode_read(
        fd,
        buf.as_mut_ptr() as usize,
        buf.len(),
    ))
}

pub fn sys_write(fd: usize, buf: &[u8]) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_vnode_write(
        fd,
        buf.as_ptr() as usize,
        buf.len(),
    ))
}

pub fn sys_lseek(fd: usize, offset: i64, whence: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_vnode_seek(
        fd,
        offset as usize,
        whence,
    ))
}

pub fn sys_stat(fd: usize, buf: *mut u8, buf_len: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_vnode_stat(
        fd,
        buf as usize,
        buf_len,
    ))
}

pub fn sys_readdir(fd: usize, buf: *mut u8, buf_len: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_vnode_readdir(
        fd,
        buf as usize,
        buf_len,
    ))
}

pub fn sys_unlink(path: &[u8]) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_vnode_unlink(
        path.as_ptr() as usize,
        path.len(),
    ))
}

pub fn sys_mkdir(path: &[u8], mode: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_vnode_mkdir(
        path.as_ptr() as usize,
        path.len(),
        mode,
    ))
}

// ---- Memory ------------------------------------------------------------------

pub fn sys_mmap(
    addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    fd: usize,
) -> Result<*mut u8, Errno> {
    let ret = hadron_syscall::wrappers::sys_mem_map(addr, len, prot, flags, fd);
    if ret < 0 {
        Err(Errno((-ret) as i32))
    } else {
        Ok(ret as usize as *mut u8)
    }
}

pub fn sys_munmap(addr: *mut u8, len: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_mem_unmap(addr as usize, len))
}

pub fn sys_mprotect(addr: *mut u8, len: usize, prot: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_mem_protect(
        addr as usize,
        len,
        prot,
    ))
}

pub fn sys_brk(addr: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_mem_brk(addr))
}

/// Clone the current task (create a thread).
///
/// Returns the child TID in the parent and 0 in the child.
///
/// # Safety
///
/// `stack_ptr` must be a valid top-of-stack pointer. `tls_ptr` must point to
/// a valid Thread Control Block whose first field is a self-pointer.
pub unsafe fn sys_task_clone(
    flags: usize,
    stack_ptr: usize,
    tls_ptr: usize,
) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_task_clone(
        flags, stack_ptr, tls_ptr,
    ))
}

// ---- Query extensions -------------------------------------------------------

/// `QUERY_VMAPS`: returns the number of bytes written into `buf` on success.
///
/// `buf` must be a pointer to an array large enough to hold all `VmapEntry`
/// structs. Use a generous size (e.g. 4096 bytes = ~128 entries).
pub fn sys_query_vmaps(buf: *mut u8, buf_len: usize) -> Result<usize, Errno> {
    let n = hadron_syscall::wrappers::sys_query(
        hadron_syscall::QUERY_VMAPS as usize,
        0,
        buf as usize,
        buf_len,
    );
    check(n)
}

/// `QUERY_CPU_INFO`: fills `buf` with a `CpuInfo` struct.
pub fn sys_query_cpu_info(buf: *mut u8, buf_len: usize) -> Result<usize, Errno> {
    let n = hadron_syscall::wrappers::sys_query(
        hadron_syscall::QUERY_CPU_INFO as usize,
        0,
        buf as usize,
        buf_len,
    );
    check(n)
}

// ---- Time / Events -----------------------------------------------------------

pub fn sys_clock_gettime(clockid: usize, tp: *mut u8) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_clock_gettime(
        clockid,
        tp as usize,
    ))
}

pub fn sys_nanosleep(req: *const u8, rem: *mut u8) -> Result<(), Errno> {
    // Hadron uses clock_nanosleep with CLOCK_MONOTONIC, flags=0
    check_unit(hadron_syscall::wrappers::sys_clock_nanosleep(
        0, // CLOCK_MONOTONIC
        0, // flags
        req as usize,
        rem as usize,
    ))
}

pub fn sys_futex(
    addr: *mut u32,
    op: usize,
    val: usize,
    timeout: *const u8,
) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_futex(
        addr as usize,
        op,
        val,
        timeout as usize,
    ))
}

pub fn sys_poll(fds: *mut u8, nfds: usize, timeout_ms: isize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_event_wait_many(
        fds as usize,
        nfds,
        timeout_ms as usize,
    ))
}

// ---- Socket ------------------------------------------------------------------

/// Create a new socket. Returns new fd on success.
pub fn sys_socket(domain: usize, type_: usize, protocol: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_socket(
        domain, type_, protocol,
    ))
}

/// Bind a socket to an address.
pub fn sys_bind(fd: usize, addr_ptr: usize, addr_len: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_bind(fd, addr_ptr, addr_len))
}

/// Mark a bound socket as listening.
pub fn sys_listen(fd: usize, backlog: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_listen(fd, backlog))
}

/// Accept a connection on a listening socket. Returns new fd on success.
pub fn sys_accept(fd: usize, addr_ptr: usize, addr_len_ptr: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_accept(
        fd,
        addr_ptr,
        addr_len_ptr,
    ))
}

/// Connect a socket to a peer.
pub fn sys_connect(fd: usize, addr_ptr: usize, addr_len: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_connect(
        fd, addr_ptr, addr_len,
    ))
}

/// Send a message on a connected socket. Returns bytes sent on success.
pub fn sys_sendmsg(fd: usize, msg_ptr: usize, flags: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_sendmsg(fd, msg_ptr, flags))
}

/// Receive a message from a connected socket. Returns bytes received on success.
pub fn sys_recvmsg(fd: usize, msg_ptr: usize, flags: usize) -> Result<usize, Errno> {
    check(hadron_syscall::wrappers::sys_recvmsg(fd, msg_ptr, flags))
}

/// Shut down part or all of a socket connection.
pub fn sys_shutdown(fd: usize, how: usize) -> Result<(), Errno> {
    check_unit(hadron_syscall::wrappers::sys_shutdown(fd, how))
}
