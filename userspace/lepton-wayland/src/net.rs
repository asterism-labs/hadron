//! Unix-domain socket helpers for the Wayland protocol.
//!
//! Provides create/bind/listen/accept/connect/send/recv primitives that mirror
//! POSIX semantics and are safe to use from a `no_std` binary. Used by both
//! the compositor (server) and display client.

use lepton_syslib::hadron_syscall::{AF_UNIX, SCM_RIGHTS, SOL_SOCKET, wrappers};

// -- POSIX structs -----------------------------------------------------------

/// `struct sockaddr_un` — path-based Unix domain socket address.
#[repr(C)]
pub struct SockaddrUn {
    /// AF_UNIX = 1
    pub sun_family: u16,
    /// Null-terminated path (108 bytes).
    pub sun_path: [u8; 108],
}

/// `struct iovec` — scatter/gather I/O descriptor (Linux x86-64, 16 bytes).
#[repr(C)]
pub struct Iovec {
    pub iov_base: usize, // *mut void
    pub iov_len: usize,
}

/// `struct msghdr` — message header for sendmsg/recvmsg (Linux x86-64, 56 bytes).
#[repr(C)]
pub struct MsgHdr {
    pub msg_name: usize,       // offset  0
    pub msg_namelen: u32,      // offset  8
    pub _pad0: u32,            // offset 12
    pub msg_iov: usize,        // offset 16
    pub msg_iovlen: usize,     // offset 24
    pub msg_control: usize,    // offset 32
    pub msg_controllen: usize, // offset 40
    pub msg_flags: i32,        // offset 48
    pub _pad1: u32,            // offset 52
}

/// `struct cmsghdr` — control message header (Linux x86-64, 16 bytes).
#[repr(C)]
pub struct CmsgHdr {
    pub cmsg_len: usize, // offset 0
    pub cmsg_level: i32, // offset 8
    pub cmsg_type: i32,  // offset 12
                         // data bytes follow at offset 16
}

/// `CMSG_SPACE(sizeof(int))` = 24 bytes (16-byte header + 4-byte fd + 4-byte pad).
pub const CMSG_SPACE_ONE_FD: usize = 24;

// -- Socket lifecycle --------------------------------------------------------

/// Create a `SOCK_STREAM AF_UNIX` socket. Returns fd or negative errno.
pub fn socket_create() -> isize {
    wrappers::sys_socket(AF_UNIX, 1 /* SOCK_STREAM */, 0)
}

/// Bind `fd` to `path` (null-terminated bytes, not including the null).
///
/// Returns 0 on success or negative errno.
pub fn socket_bind(fd: usize, path: &[u8]) -> isize {
    let mut addr = SockaddrUn {
        sun_family: AF_UNIX as u16,
        sun_path: [0u8; 108],
    };
    let n = path.len().min(107);
    addr.sun_path[..n].copy_from_slice(&path[..n]);
    // addrlen = sizeof(sun_family) + strlen(sun_path) + 1 null byte
    let addr_len = 2 + n + 1;
    wrappers::sys_bind(fd, &addr as *const SockaddrUn as usize, addr_len)
}

/// Connect `fd` to a listening socket at `path`.
///
/// Returns 0 on success or negative errno.
pub fn socket_connect(fd: usize, path: &[u8]) -> isize {
    let mut addr = SockaddrUn {
        sun_family: AF_UNIX as u16,
        sun_path: [0u8; 108],
    };
    let n = path.len().min(107);
    addr.sun_path[..n].copy_from_slice(&path[..n]);
    let addr_len = 2 + n + 1;
    wrappers::sys_connect(fd, &addr as *const SockaddrUn as usize, addr_len)
}

/// Mark `fd` as passive listener with `backlog` pending slots.
pub fn socket_listen(fd: usize, backlog: usize) -> isize {
    wrappers::sys_listen(fd, backlog)
}

/// Accept one incoming connection on `fd`. Returns new fd or negative errno.
///
/// Passes null for address storage (we don't need the peer address).
pub fn socket_accept(fd: usize) -> isize {
    wrappers::sys_accept(fd, 0, 0)
}

// -- Send/receive ------------------------------------------------------------

/// Write all of `buf` to `fd`, retrying on short writes.
///
/// Returns `true` if all bytes were sent.
pub fn send_all(fd: usize, buf: &[u8]) -> bool {
    let mut sent = 0usize;
    while sent < buf.len() {
        let iov = Iovec {
            iov_base: buf[sent..].as_ptr() as usize,
            iov_len: buf.len() - sent,
        };
        let mut msg = MsgHdr {
            msg_name: 0,
            msg_namelen: 0,
            _pad0: 0,
            msg_iov: &iov as *const Iovec as usize,
            msg_iovlen: 1,
            msg_control: 0,
            msg_controllen: 0,
            msg_flags: 0,
            _pad1: 0,
        };
        let ret = wrappers::sys_sendmsg(fd, &mut msg as *mut MsgHdr as usize, 0);
        if ret <= 0 {
            return false;
        }
        sent += ret as usize;
    }
    true
}

/// Send `buf` with a single SCM_RIGHTS file descriptor attached.
///
/// Returns `true` if the message was sent successfully.
pub fn send_with_fd(fd: usize, buf: &[u8], pass_fd: i32) -> bool {
    let iov = Iovec {
        iov_base: buf.as_ptr() as usize,
        iov_len: buf.len(),
    };

    // Build control message: cmsghdr + fd
    let mut cmsg_buf = [0u8; CMSG_SPACE_ONE_FD];

    // SAFETY: writing the cmsghdr struct fields into the aligned buffer.
    let hdr = CmsgHdr {
        cmsg_len: 16 + 4, // sizeof(cmsghdr) + sizeof(int)
        cmsg_level: SOL_SOCKET as i32,
        cmsg_type: SCM_RIGHTS as i32,
    };
    // SAFETY: cmsg_buf is large enough and properly aligned for this write.
    unsafe {
        core::ptr::write_unaligned(cmsg_buf.as_mut_ptr().cast::<CmsgHdr>(), hdr);
    }
    // Write the fd at offset 16
    let fd_bytes = pass_fd.to_le_bytes();
    cmsg_buf[16..20].copy_from_slice(&fd_bytes);

    let mut msg = MsgHdr {
        msg_name: 0,
        msg_namelen: 0,
        _pad0: 0,
        msg_iov: &iov as *const Iovec as usize,
        msg_iovlen: 1,
        msg_control: cmsg_buf.as_ptr() as usize,
        msg_controllen: CMSG_SPACE_ONE_FD,
        msg_flags: 0,
        _pad1: 0,
    };

    let ret = wrappers::sys_sendmsg(fd, &mut msg as *mut MsgHdr as usize, 0);
    ret > 0
}

/// Receive up to `buf.len()` bytes from `fd`.
///
/// Also drains one SCM_RIGHTS fd if present in the ancillary data.
/// Returns `(bytes_read, received_fd)` where `received_fd` is -1 if no fd.
pub fn recv_with_fd(fd: usize, buf: &mut [u8]) -> (isize, i32) {
    let iov = Iovec {
        iov_base: buf.as_mut_ptr() as usize,
        iov_len: buf.len(),
    };
    let mut cmsg_buf = [0u8; CMSG_SPACE_ONE_FD];
    let mut msg = MsgHdr {
        msg_name: 0,
        msg_namelen: 0,
        _pad0: 0,
        msg_iov: &iov as *const Iovec as usize,
        msg_iovlen: 1,
        msg_control: cmsg_buf.as_mut_ptr() as usize,
        msg_controllen: CMSG_SPACE_ONE_FD,
        msg_flags: 0,
        _pad1: 0,
    };

    let ret = wrappers::sys_recvmsg(fd, &mut msg as *mut MsgHdr as usize, 0);
    if ret <= 0 {
        return (ret, -1);
    }

    // Inspect ancillary data for a single SCM_RIGHTS fd.
    let ctrl_len = unsafe { core::ptr::read_volatile(&msg.msg_controllen) };
    let mut recv_fd: i32 = -1;
    if ctrl_len >= 16 + 4 {
        // SAFETY: kernel wrote valid cmsghdr + fd bytes into cmsg_buf
        let hdr = unsafe { &*(cmsg_buf.as_ptr().cast::<CmsgHdr>()) };
        if hdr.cmsg_level == SOL_SOCKET as i32 && hdr.cmsg_type == SCM_RIGHTS as i32 {
            let fd_bytes = &cmsg_buf[16..20];
            recv_fd = i32::from_le_bytes([fd_bytes[0], fd_bytes[1], fd_bytes[2], fd_bytes[3]]);
        }
    }

    (ret, recv_fd)
}
