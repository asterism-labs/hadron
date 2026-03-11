//! Network syscall stubs.
//!
//! All network syscalls return `ENOSYS` until the network stack is
//! implemented in a later phase.

use hadron_syscall::ENOSYS;

/// `SYS_NET_SOCKET` — stub.
pub fn sys_net_socket(_domain: usize, _sock_type: usize, _protocol: usize) -> isize {
    -ENOSYS
}

/// `SYS_NET_BIND` — stub.
pub fn sys_net_bind(_fd: usize, _addr_ptr: usize, _addr_len: usize) -> isize {
    -ENOSYS
}

/// `SYS_NET_LISTEN` — stub.
pub fn sys_net_listen(_fd: usize, _backlog: usize) -> isize {
    -ENOSYS
}

/// `SYS_NET_ACCEPT` — stub.
pub fn sys_net_accept(_fd: usize, _addr_ptr: usize, _addr_len_ptr: usize) -> isize {
    -ENOSYS
}

/// `SYS_NET_CONNECT` — stub.
pub fn sys_net_connect(_fd: usize, _addr_ptr: usize, _addr_len: usize) -> isize {
    -ENOSYS
}

/// `SYS_NET_SENDMSG` — stub.
pub fn sys_net_sendmsg(_fd: usize, _msg_ptr: usize, _flags: usize) -> isize {
    -ENOSYS
}

/// `SYS_NET_RECVMSG` — stub.
pub fn sys_net_recvmsg(_fd: usize, _msg_ptr: usize, _flags: usize) -> isize {
    -ENOSYS
}

/// `SYS_NET_SHUTDOWN` — stub.
pub fn sys_net_shutdown(_fd: usize, _how: usize) -> isize {
    -ENOSYS
}
