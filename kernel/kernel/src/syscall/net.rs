//! AF_UNIX socket syscall handlers.
//!
//! Implements the eight socket syscalls (socket, bind, listen, accept, connect,
//! sendmsg, recvmsg, shutdown) on top of the [`UnixSocket`] inode type.
//!
//! [`UnixSocket`]: crate::net::unix::UnixSocket

extern crate alloc;

use alloc::sync::Arc;
use alloc::vec;

use crate::fs::file::OpenFlags;
use crate::fs::{Inode, InodeType, try_poll_immediate};
use crate::id::Fd;
use crate::net::unix::UnixSocket;
use crate::syscall::userptr::UserSlice;
use crate::syscall::{AF_UNIX, EBADF, ECONNREFUSED, EFAULT, EINVAL, ENOTSOCK, SOCK_STREAM};

// ── POSIX struct layouts (Linux x86-64 ABI) ──────────────────────────────────

/// `struct sockaddr_un` (POSIX, 110 bytes on x86-64).
///
/// `sun_family` is 2 bytes; `sun_path` is the null-terminated socket path.
#[repr(C)]
struct SockaddrUn {
    sun_family: u16,
    sun_path: [u8; 108],
}

/// `struct iovec` (POSIX, 16 bytes on x86-64).
#[repr(C)]
struct Iovec {
    iov_base: u64,
    iov_len: u64,
}

/// `struct msghdr` (Linux x86-64 ABI, 56 bytes).
#[repr(C)]
struct MsgHdr {
    msg_name: u64,
    msg_namelen: u32,
    _pad0: u32,
    msg_iov: u64,
    msg_iovlen: u64,
    msg_control: u64,
    msg_controllen: u64,
    msg_flags: u32,
    _pad1: u32,
}

/// `struct cmsghdr` (Linux x86-64 ABI, 16 bytes; `cmsg_len` is `size_t`).
#[repr(C)]
struct CmsgHdr {
    cmsg_len: u64,
    cmsg_level: i32,
    cmsg_type: i32,
}

/// `CMSG_SPACE(sizeof(int))` — space for a single fd in an SCM_RIGHTS cmsg.
const CMSG_SPACE_1FD: usize = 24;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Look up an fd and clone its inode, returning `-EBADF` on failure.
fn fd_inode(fd: Fd) -> Result<Arc<dyn Inode>, isize> {
    crate::proc::ProcessTable::with_current(|process| {
        let fd_table = process.fd_table.lock();
        let Some(file) = fd_table.get(fd) else {
            return Err(-EBADF);
        };
        Ok(file.inode.clone())
    })
}

/// Read the socket path out of a `struct sockaddr_un` at `addr_ptr`.
fn read_sockaddr_un_path(addr_ptr: usize, addr_len: usize) -> Result<alloc::string::String, isize> {
    // Minimum: 2 bytes sun_family + at least 1 path byte + null terminator.
    if addr_ptr == 0 || addr_len < 3 {
        return Err(-EINVAL);
    }
    let read_len = addr_len.min(core::mem::size_of::<SockaddrUn>());
    let Ok(slice) = UserSlice::new(addr_ptr, read_len) else {
        return Err(-EFAULT);
    };
    // SAFETY: UserSlice validated that [addr_ptr, addr_ptr+read_len) is in user space.
    let bytes = unsafe { slice.as_slice() };
    // Skip sun_family (2 bytes), then find the null-terminated path.
    let path_bytes = &bytes[2..];
    let path_len = path_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(path_bytes.len());
    let Ok(path) = core::str::from_utf8(&path_bytes[..path_len]) else {
        return Err(-EINVAL);
    };
    Ok(alloc::string::String::from(path))
}

/// Read a `struct msghdr` from user space at `msg_ptr`.
fn read_msghdr(msg_ptr: usize) -> Result<MsgHdr, isize> {
    if msg_ptr == 0 {
        return Err(-EFAULT);
    }
    let Ok(slice) = UserSlice::new(msg_ptr, core::mem::size_of::<MsgHdr>()) else {
        return Err(-EFAULT);
    };
    // SAFETY: UserSlice validated pointer range; MsgHdr is repr(C) with no
    // invalid bit patterns (all fields are integers).
    let hdr: MsgHdr =
        unsafe { core::ptr::read_unaligned(slice.as_slice().as_ptr() as *const MsgHdr) };
    Ok(hdr)
}

/// Read `iovec[0]` from the iov array pointer stored in `msghdr`.
///
/// Returns `(iov_base, iov_len)`.
fn read_iov0(msghdr: &MsgHdr) -> Result<(usize, usize), isize> {
    if msghdr.msg_iovlen == 0 || msghdr.msg_iov == 0 {
        return Err(-EINVAL);
    }
    let Ok(slice) = UserSlice::new(msghdr.msg_iov as usize, core::mem::size_of::<Iovec>()) else {
        return Err(-EFAULT);
    };
    // SAFETY: Same as read_msghdr.
    let iov: Iovec =
        unsafe { core::ptr::read_unaligned(slice.as_slice().as_ptr() as *const Iovec) };
    Ok((iov.iov_base as usize, iov.iov_len as usize))
}

/// Process ancillary data from a `sendmsg` call, enqueueing `SCM_RIGHTS` fds.
///
/// Iterates the cmsg chain and for each `SOL_SOCKET` / `SCM_RIGHTS` entry,
/// looks up each fd's inode and enqueues it on `inode` via
/// [`Inode::enqueue_send_fd`].
fn process_send_ancillary(inode: &Arc<dyn Inode>, msg_control: u64, msg_controllen: u64) {
    if msg_control == 0 || msg_controllen == 0 {
        return;
    }
    let ctrl_ptr = msg_control as usize;
    let ctrl_len = msg_controllen as usize;
    let Ok(slice) = UserSlice::new(ctrl_ptr, ctrl_len) else {
        return;
    };
    // SAFETY: UserSlice validated pointer range.
    let ctrl_bytes = unsafe { slice.as_slice() };

    let cmsg_hdr_size = core::mem::size_of::<CmsgHdr>(); // 16
    let mut offset = 0usize;

    while offset + cmsg_hdr_size <= ctrl_len {
        // SAFETY: Bounds checked above; CmsgHdr is repr(C) with no invalid
        // bit patterns.
        let hdr: CmsgHdr =
            unsafe { core::ptr::read_unaligned(ctrl_bytes.as_ptr().add(offset) as *const CmsgHdr) };
        let cmsg_len = hdr.cmsg_len as usize;
        if cmsg_len < cmsg_hdr_size || offset + cmsg_len > ctrl_len {
            break;
        }
        // SOL_SOCKET (1) + SCM_RIGHTS (1): fd array follows header at offset 16.
        if hdr.cmsg_level == 1 && hdr.cmsg_type == 1 {
            let data_len = cmsg_len - cmsg_hdr_size;
            let fd_count = data_len / 4; // sizeof(int) == 4
            for i in 0..fd_count {
                let fd_off = offset + cmsg_hdr_size + i * 4;
                if fd_off + 4 > ctrl_len {
                    break;
                }
                // SAFETY: Bounds checked above; i32 has no invalid bit patterns.
                let fd_num: i32 = unsafe {
                    core::ptr::read_unaligned(ctrl_bytes.as_ptr().add(fd_off) as *const i32)
                };
                if fd_num >= 0 {
                    let send_fd_inode = crate::proc::ProcessTable::with_current(|p| {
                        p.fd_table
                            .lock()
                            .get(Fd::new(fd_num as u32))
                            .map(|f| f.inode.clone())
                    });
                    if let Some(fi) = send_fd_inode {
                        inode.enqueue_send_fd(fi);
                    }
                }
            }
        }
        // Advance by CMSG_ALIGN(cmsg_len) = (cmsg_len + 7) & !7.
        let aligned = ((cmsg_len + 7) & !7).max(cmsg_hdr_size);
        offset += aligned;
    }
}

// ── trap_accept ───────────────────────────────────────────────────────────────

/// Trigger a `TRAP_ACCEPT` longjmp back to `process_task` for async accept.
///
/// Sets `AcceptState` fd, restores kernel CR3 and GS, then calls
/// `restore_kernel_context` — never returns.
pub(super) fn trap_accept(fd: Fd) -> ! {
    use crate::arch::x86_64::registers::control::Cr3;
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
    use crate::arch::x86_64::userspace::restore_kernel_context;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();

    // SAFETY: Restoring kernel CR3 and GS bases is the standard pattern for
    // returning from user-space context to the kernel async loop.
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    crate::proc::AcceptState::set_fd(fd);
    crate::proc::TrapContext::set_trap_reason(crate::proc::TrapReason::Accept);

    let saved_rsp = crate::proc::TrapContext::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save,
    // still valid on the executor stack.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}

/// Trigger a `TRAP_IO` read longjmp, with optional `recvmsg` cmsg parameters.
///
/// If `cmsg_ptr != 0`, `process_task` will dequeue pending `SCM_RIGHTS` fds
/// after the read and write them into the caller's cmsg buffer.
fn trap_recvmsg(
    fd: Fd,
    buf_ptr: usize,
    buf_len: usize,
    cmsg_ptr: usize,
    cmsg_len: usize,
    msg_ptr: usize,
) -> ! {
    use crate::arch::x86_64::registers::control::Cr3;
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
    use crate::arch::x86_64::userspace::restore_kernel_context;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();

    // SAFETY: Same as trap_accept.
    unsafe {
        Cr3::write(kernel_cr3);
        let percpu = IA32_GS_BASE.read();
        IA32_KERNEL_GS_BASE.write(percpu);
    }

    // set_params clears cmsg; set_recvmsg_params re-establishes them.
    crate::proc::IoState::set_params(fd, buf_ptr, buf_len, false);
    crate::proc::IoState::set_recvmsg_params(cmsg_ptr, cmsg_len, msg_ptr);
    crate::proc::TrapContext::set_trap_reason(crate::proc::TrapReason::Io);

    let saved_rsp = crate::proc::TrapContext::saved_kernel_rsp();
    // SAFETY: saved_rsp is the kernel RSP saved by enter_userspace_save.
    unsafe {
        restore_kernel_context(saved_rsp);
    }
}

// ── Syscall handlers ──────────────────────────────────────────────────────────

/// `sys_socket` — create a new `AF_UNIX` `SOCK_STREAM` socket.
///
/// Returns the new file descriptor on success, or negated errno on failure.
#[expect(clippy::cast_possible_wrap, reason = "fd numbers are small")]
pub(super) fn sys_socket(domain: usize, type_: usize, _protocol: usize) -> isize {
    if domain != AF_UNIX || type_ != SOCK_STREAM {
        return -EINVAL;
    }
    let socket: Arc<dyn Inode> = UnixSocket::new();
    let fd = crate::proc::ProcessTable::with_current(|p| {
        p.fd_table
            .lock()
            .open(socket, OpenFlags::READ | OpenFlags::WRITE)
    });
    fd.as_u32() as isize
}

/// `sys_bind` — bind a socket to a filesystem path.
///
/// `addr_ptr` points to a `struct sockaddr_un`. Returns 0 on success.
pub(super) fn sys_bind(fd: usize, addr_ptr: usize, addr_len: usize) -> isize {
    let path = match read_sockaddr_un_path(addr_ptr, addr_len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let inode = match fd_inode(Fd::new(fd as u32)) {
        Ok(i) => i,
        Err(e) => return e,
    };
    if inode.inode_type() != InodeType::Socket {
        return -ENOTSOCK;
    }
    match inode.unix_bind(&path) {
        Ok(()) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_listen` — mark a bound socket as listening for connections.
///
/// `backlog` is the maximum number of pending connections. Returns 0 on success.
pub(super) fn sys_listen(fd: usize, backlog: usize) -> isize {
    let inode = match fd_inode(Fd::new(fd as u32)) {
        Ok(i) => i,
        Err(e) => return e,
    };
    if inode.inode_type() != InodeType::Socket {
        return -ENOTSOCK;
    }
    match inode.unix_listen(backlog) {
        Ok(()) => 0,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_accept` — accept a connection on a listening socket.
///
/// Blocks until a connection is available. `addr_ptr` and `addr_len_ptr` are
/// accepted but ignored for Phase 1 (AF_UNIX only). Returns the new fd.
pub(super) fn sys_accept(fd: usize, _addr_ptr: usize, _addr_len_ptr: usize) -> isize {
    let fd = Fd::new(fd as u32);
    let inode = match fd_inode(fd) {
        Ok(i) => i,
        Err(e) => return e,
    };
    if inode.inode_type() != InodeType::Socket {
        return -ENOTSOCK;
    }
    // Check if a connection is immediately available to avoid a longjmp.
    match crate::fs::try_poll_immediate(inode.accept_connection()) {
        Some(Ok(new_inode)) => {
            let new_fd = crate::proc::ProcessTable::with_current(|p| {
                p.fd_table
                    .lock()
                    .open(new_inode, OpenFlags::READ | OpenFlags::WRITE)
            });
            #[expect(clippy::cast_possible_wrap, reason = "fd numbers are small")]
            {
                new_fd.as_u32() as isize
            }
        }
        Some(Err(e)) => -e.to_errno(),
        None => {
            // No connection ready — block in process_task.
            drop(inode);
            trap_accept(fd)
        }
    }
}

/// `sys_connect` — connect a socket to a listening peer.
///
/// `addr_ptr` points to a `struct sockaddr_un` with the target path.
/// Returns 0 on success, or negated errno on failure.
pub(super) fn sys_connect(fd: usize, addr_ptr: usize, addr_len: usize) -> isize {
    let path = match read_sockaddr_un_path(addr_ptr, addr_len) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let inode = match fd_inode(Fd::new(fd as u32)) {
        Ok(i) => i,
        Err(e) => return e,
    };
    if inode.inode_type() != InodeType::Socket {
        return -ENOTSOCK;
    }
    match inode.unix_connect(&path) {
        Ok(()) => 0,
        // Backlog full — translate IoError → ECONNREFUSED.
        Err(crate::fs::FsError::IoError) => -ECONNREFUSED,
        Err(e) => -e.to_errno(),
    }
}

/// `sys_sendmsg` — send a message on a connected socket.
///
/// Processes `SCM_RIGHTS` ancillary data (enqueuing fds) then writes the
/// data from `iov[0]`. Blocks via `TRAP_IO` if the send buffer is full.
pub(super) fn sys_sendmsg(fd: usize, msg_ptr: usize, _flags: usize) -> isize {
    let fd = Fd::new(fd as u32);

    let msghdr = match read_msghdr(msg_ptr) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let (iov_base, iov_len) = match read_iov0(&msghdr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let inode = match fd_inode(fd) {
        Ok(i) => i,
        Err(e) => return e,
    };
    if inode.inode_type() != InodeType::Socket {
        return -ENOTSOCK;
    }

    // Enqueue SCM_RIGHTS fds before writing data.
    process_send_ancillary(&inode, msghdr.msg_control, msghdr.msg_controllen);

    // Copy iov data into a kernel buffer for the synchronous attempt.
    let mut kbuf = vec![0u8; iov_len];
    let Ok(uslice) = UserSlice::new(iov_base, iov_len) else {
        return -EFAULT;
    };
    // SAFETY: UserSlice validated the user pointer range.
    let src = unsafe { uslice.as_slice() };
    kbuf.copy_from_slice(src);

    // Try synchronous write first.
    match try_poll_immediate(inode.write(0, &kbuf)) {
        Some(Ok(n)) => {
            #[expect(clippy::cast_possible_wrap, reason = "byte counts are small")]
            {
                n as isize
            }
        }
        Some(Err(e)) => -e.to_errno(),
        None => {
            // Buffer full — fall back to the async TRAP_IO path.
            // The fds are already enqueued and will be paired with the data.
            drop(inode);
            super::vfs::trap_io(fd, iov_base, iov_len, true)
        }
    }
}

/// `sys_recvmsg` — receive a message from a connected socket.
///
/// Tries a synchronous read first. If data is available immediately, fills
/// `iov[0]` and dequeues any `SCM_RIGHTS` fds into `msg_control`. If no data
/// is ready, blocks via `TRAP_IO`; the async path also handles cmsg fds.
pub(super) fn sys_recvmsg(fd: usize, msg_ptr: usize, _flags: usize) -> isize {
    let fd = Fd::new(fd as u32);

    let msghdr = match read_msghdr(msg_ptr) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let (iov_base, iov_len) = match read_iov0(&msghdr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let inode = match fd_inode(fd) {
        Ok(i) => i,
        Err(e) => return e,
    };
    if inode.inode_type() != InodeType::Socket {
        return -ENOTSOCK;
    }

    // Try synchronous read.
    let mut kbuf = vec![0u8; iov_len];
    let n = match try_poll_immediate(inode.read(0, &mut kbuf)) {
        Some(Ok(n)) => n,
        Some(Err(e)) => return -e.to_errno(),
        None => {
            // No data yet — block and let process_task handle cmsg too.
            drop(inode);
            return trap_recvmsg(
                fd,
                iov_base,
                iov_len,
                msghdr.msg_control as usize,
                msghdr.msg_controllen as usize,
                msg_ptr,
            );
        }
    };

    // Copy kernel buffer to user iov.
    let Ok(uslice) = UserSlice::new(iov_base, n) else {
        return -EFAULT;
    };
    // SAFETY: UserSlice validated the user pointer range.
    let dst = unsafe { uslice.as_mut_slice() };
    dst.copy_from_slice(&kbuf[..n]);

    // Dequeue SCM_RIGHTS fds into msg_control.
    let cmsg_ptr = msghdr.msg_control as usize;
    let cmsg_max = msghdr.msg_controllen as usize;
    let mut written_cmsgs = 0usize;

    if cmsg_ptr != 0 && cmsg_max >= CMSG_SPACE_1FD {
        while written_cmsgs + CMSG_SPACE_1FD <= cmsg_max {
            let Some(recv_inode) = inode.dequeue_recv_fd() else {
                break;
            };
            let new_fd = crate::proc::ProcessTable::with_current(|p| {
                p.fd_table
                    .lock()
                    .open(recv_inode, OpenFlags::READ | OpenFlags::WRITE)
            });
            let cmsg_offset = cmsg_ptr + written_cmsgs;
            // Write cmsg: [cmsg_len:u64=20][SOL_SOCKET:i32=1][SCM_RIGHTS:i32=1][fd:i32]
            if let Ok(sl) = UserSlice::new(cmsg_offset, 20) {
                // SAFETY: UserSlice validated pointer range.
                let buf = unsafe { sl.as_mut_slice() };
                buf[0..8].copy_from_slice(&20u64.to_le_bytes());
                buf[8..12].copy_from_slice(&1i32.to_le_bytes());
                buf[12..16].copy_from_slice(&1i32.to_le_bytes());
                buf[16..20].copy_from_slice(&(new_fd.as_u32() as i32).to_le_bytes());
            }
            written_cmsgs += CMSG_SPACE_1FD;
        }
        // Update msg_controllen at byte offset 40 of the msghdr.
        if let Ok(sl) = UserSlice::new(msg_ptr + 40, 8) {
            // SAFETY: UserSlice validated pointer range.
            let buf = unsafe { sl.as_mut_slice() };
            buf.copy_from_slice(&(written_cmsgs as u64).to_le_bytes());
        }
    }

    #[expect(clippy::cast_possible_wrap, reason = "byte counts are small")]
    {
        n as isize
    }
}

/// `sys_shutdown` — shut down part or all of a full-duplex connection.
///
/// `how` is `SHUT_RD` (0), `SHUT_WR` (1), or `SHUT_RDWR` (2).
/// Returns 0 on success.
pub(super) fn sys_shutdown(fd: usize, how: usize) -> isize {
    let inode = match fd_inode(Fd::new(fd as u32)) {
        Ok(i) => i,
        Err(e) => return e,
    };
    if inode.inode_type() != InodeType::Socket {
        return -ENOTSOCK;
    }
    if how > 2 {
        return -EINVAL;
    }
    inode.unix_shutdown(how as u8);
    0
}
