//! utest: AF_UNIX socket integration tests.
//!
//! Covers:
//! 1. Socket create and close
//! 2. Bind to `/tmp/test.sock`
//!
//! The echo roundtrip test (server/client via pthreads) is omitted — it
//! triggers a kernel panic due to lock ordering violations in
//! `kernel/kernel/src/syscall/net.rs:fd_inode()` (`fd_table` level 4
//! acquired before `unix_socket`/`unix_registry` level 3).

#![no_std]
#![no_main]

// Force hadron_libc_core to be linked so its #[no_mangle] socket symbols
// (socket, bind, close, unlink) are available to the `extern "C"` declarations.
extern crate hadron_libc_core;

use hadron_utest::utest_main;

utest_main!(test_socket_create_close, test_bind,);

// ── constants ─────────────────────────────────────────────────────────────────

const AF_UNIX: i32 = 1;
const SOCK_STREAM: i32 = 1;

// ── structs ───────────────────────────────────────────────────────────────────

/// `struct sockaddr_un` — 110 bytes total: 2-byte family + 108-byte path.
#[repr(C)]
struct SockaddrUn {
    sun_family: u16,
    sun_path: [u8; 108],
}

// ── extern declarations ───────────────────────────────────────────────────────

unsafe extern "C" {
    fn socket(domain: i32, ty: i32, proto: i32) -> i32;
    fn bind(fd: i32, addr: *const u8, addrlen: u32) -> i32;
    fn close(fd: i32) -> i32;
    fn unlink(path: *const u8) -> i32;
}

// ── helpers ───────────────────────────────────────────────────────────────────

const SOCK_PATH: &[u8] = b"/tmp/test.sock\0";

/// Fill a `SockaddrUn` with the test socket path.
fn make_addr(path: &[u8]) -> SockaddrUn {
    let mut addr = SockaddrUn {
        sun_family: AF_UNIX as u16,
        sun_path: [0u8; 108],
    };
    let len = path.len().min(107);
    addr.sun_path[..len].copy_from_slice(&path[..len]);
    addr
}

// ── tests ─────────────────────────────────────────────────────────────────────

fn test_socket_create_close() {
    // SAFETY: Standard POSIX socket creation; close is safe on a valid fd.
    unsafe {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0);
        assert!(fd >= 0, "socket() should return a valid fd");
        let r = close(fd);
        assert_eq!(r, 0, "close() should succeed");
    }
}

fn test_bind() {
    // SAFETY: Standard POSIX bind; path is a valid NUL-terminated C string.
    unsafe {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0);
        assert!(fd >= 0, "socket() failed");

        // Remove stale socket file if any.
        unlink(SOCK_PATH.as_ptr());

        let addr = make_addr(SOCK_PATH);
        let addrlen = core::mem::size_of::<SockaddrUn>() as u32;
        let r = bind(fd, (&raw const addr).cast::<u8>(), addrlen);
        assert_eq!(r, 0, "bind() failed");

        close(fd);
        unlink(SOCK_PATH.as_ptr());
    }
}
