//! POSIX socket API: `socket`, `bind`, `listen`, `accept`, `connect`,
//! `sendmsg`, `recvmsg`, `shutdown`.
//!
//! Only `AF_UNIX` / `SOCK_STREAM` is supported.  The struct layouts are
//! identical to the Linux x86-64 ABI so standard `<sys/socket.h>` headers
//! compile against this implementation without modification.

use crate::{errno, sys};

// ── POSIX / Linux x86-64 struct layouts ─────────────────────────────────────

/// `sa_family_t` — socket address family tag.
pub type SaFamily = u16;

/// `socklen_t` — socket address length.
pub type Socklen = u32;

/// `AF_UNIX` address family.
pub const AF_UNIX: u16 = 1;
/// `SOCK_STREAM` socket type — reliable bidirectional byte stream.
pub const SOCK_STREAM: i32 = 1;

/// `SHUT_RD` — shut down the read half.
pub const SHUT_RD: i32 = 0;
/// `SHUT_WR` — shut down the write half.
pub const SHUT_WR: i32 = 1;
/// `SHUT_RDWR` — shut down both halves.
pub const SHUT_RDWR: i32 = 2;

/// `SOL_SOCKET` — socket-level option layer.
pub const SOL_SOCKET: i32 = 1;
/// `SCM_RIGHTS` — ancillary data type for fd passing.
pub const SCM_RIGHTS: i32 = 1;

/// `struct sockaddr_un` — Unix domain socket address (110 bytes).
///
/// The `sun_path` field is a null-terminated filesystem path.
#[repr(C)]
pub struct SockaddrUn {
    /// Address family (`AF_UNIX` = 1).
    pub sun_family: SaFamily,
    /// Socket path (null-terminated).
    pub sun_path: [u8; 108],
}

/// `struct iovec` — I/O vector for scatter/gather I/O (16 bytes).
#[repr(C)]
pub struct Iovec {
    /// Pointer to data buffer.
    pub iov_base: *mut u8,
    /// Length of data buffer.
    pub iov_len: usize,
}

/// `struct msghdr` — message header for `sendmsg` / `recvmsg` (56 bytes).
#[repr(C)]
pub struct MsgHdr {
    /// Optional peer address (may be null).
    pub msg_name: *mut u8,
    /// Size of `msg_name`.
    pub msg_namelen: Socklen,
    _pad0: u32,
    /// Pointer to scatter/gather I/O vector.
    pub msg_iov: *mut Iovec,
    /// Number of entries in `msg_iov`.
    pub msg_iovlen: usize,
    /// Pointer to ancillary data buffer.
    pub msg_control: *mut u8,
    /// Size of ancillary data buffer.
    pub msg_controllen: usize,
    /// Message flags (filled in by `recvmsg`).
    pub msg_flags: i32,
    _pad1: u32,
}

/// `struct cmsghdr` — ancillary data header (16 bytes on x86-64).
///
/// Data follows immediately at offset 16 (`CMSG_DATA`).
#[repr(C)]
pub struct CmsgHdr {
    /// Total length of this cmsg including header (in bytes).
    pub cmsg_len: usize,
    /// Originating protocol level (`SOL_SOCKET` = 1).
    pub cmsg_level: i32,
    /// Protocol-specific type (`SCM_RIGHTS` = 1 for fd passing).
    pub cmsg_type: i32,
}

/// `CMSG_DATA(cmsg)` — pointer to the data portion of a `struct cmsghdr`.
///
/// # Safety
///
/// `cmsg` must point to a valid, sufficiently-sized `struct cmsghdr`.
#[inline]
pub unsafe fn cmsg_data(cmsg: *mut CmsgHdr) -> *mut u8 {
    // SAFETY: CmsgHdr is 16 bytes; data immediately follows.
    unsafe { cmsg.cast::<u8>().add(core::mem::size_of::<CmsgHdr>()) }
}

/// `CMSG_SPACE(data_len)` — total space for a cmsg with `data_len` bytes of data.
///
/// Includes the header and rounds up to the next `size_t` alignment boundary.
#[inline]
#[must_use]
pub const fn cmsg_space(data_len: usize) -> usize {
    (core::mem::size_of::<CmsgHdr>() + data_len + core::mem::size_of::<usize>() - 1)
        & !(core::mem::size_of::<usize>() - 1)
}

/// `CMSG_LEN(data_len)` — value to store in `cmsg_len` for `data_len` bytes.
#[inline]
#[must_use]
pub const fn cmsg_len(data_len: usize) -> usize {
    core::mem::size_of::<CmsgHdr>() + data_len
}

// ── C ABI wrappers ───────────────────────────────────────────────────────────

/// Create a socket.
///
/// `domain` must be `AF_UNIX` (1). `type_` must be `SOCK_STREAM` (1).
/// `protocol` should be 0.
///
/// Returns a non-negative file descriptor on success, or `-1` with `errno` set.
///
/// # Safety
///
/// Standard POSIX socket creation; no unsafe memory access.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn socket(domain: i32, type_: i32, protocol: i32) -> i32 {
    match sys::sys_socket(domain as usize, type_ as usize, protocol as usize) {
        Ok(fd) => fd as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Bind a socket to an address.
///
/// `addr` must point to a `struct sockaddr_un`. `addrlen` is its size.
///
/// Returns 0 on success, or `-1` with `errno` set.
///
/// # Safety
///
/// `addr` must be a valid pointer to a `sockaddr_un` of at least `addrlen` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bind(fd: i32, addr: *const u8, addrlen: Socklen) -> i32 {
    match sys::sys_bind(fd as usize, addr as usize, addrlen as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Mark a socket as passive (listening for connections).
///
/// `backlog` is the maximum number of pending connections.
/// Returns 0 on success, or `-1` with `errno` set.
///
/// # Safety
///
/// `fd` must refer to a bound socket.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn listen(fd: i32, backlog: i32) -> i32 {
    match sys::sys_listen(fd as usize, backlog as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Accept an incoming connection on a listening socket.
///
/// Blocks until a connection is available. `addr` and `addrlen` are ignored
/// (may be null) for Phase 1.
///
/// Returns the new file descriptor on success, or `-1` with `errno` set.
///
/// # Safety
///
/// `fd` must refer to a listening socket.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn accept(fd: i32, addr: *mut u8, addrlen: *mut Socklen) -> i32 {
    match sys::sys_accept(fd as usize, addr as usize, addrlen as usize) {
        Ok(new_fd) => new_fd as i32,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Connect a socket to a peer address.
///
/// `addr` must point to a `struct sockaddr_un`. `addrlen` is its size.
///
/// Returns 0 on success, or `-1` with `errno` set.
///
/// # Safety
///
/// `addr` must be a valid pointer to a `sockaddr_un` of at least `addrlen` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn connect(fd: i32, addr: *const u8, addrlen: Socklen) -> i32 {
    match sys::sys_connect(fd as usize, addr as usize, addrlen as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Send a message on a socket.
///
/// `msg` must point to a valid `struct msghdr`. `flags` is currently ignored.
///
/// Returns the number of bytes sent on success, or `-1` with `errno` set.
///
/// # Safety
///
/// `msg` must be a valid pointer to a `struct msghdr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sendmsg(fd: i32, msg: *const MsgHdr, flags: i32) -> isize {
    match sys::sys_sendmsg(fd as usize, msg as usize, flags as usize) {
        Ok(n) => n as isize,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Receive a message from a socket.
///
/// `msg` must point to a valid `struct msghdr` with pre-allocated `msg_iov` and
/// optional `msg_control` buffers. `flags` is currently ignored.
///
/// Returns the number of bytes received on success, or `-1` with `errno` set.
///
/// # Safety
///
/// `msg` must be a valid pointer to an initialised `struct msghdr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn recvmsg(fd: i32, msg: *mut MsgHdr, flags: i32) -> isize {
    match sys::sys_recvmsg(fd as usize, msg as usize, flags as usize) {
        Ok(n) => n as isize,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Shut down part or all of a full-duplex connection.
///
/// `how` is `SHUT_RD` (0), `SHUT_WR` (1), or `SHUT_RDWR` (2).
///
/// Returns 0 on success, or `-1` with `errno` set.
///
/// # Safety
///
/// `fd` must refer to a connected socket.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shutdown(fd: i32, how: i32) -> i32 {
    match sys::sys_shutdown(fd as usize, how as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}
