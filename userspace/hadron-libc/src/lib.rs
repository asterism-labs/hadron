//! Hadron libc — thin C ABI shell producing `libc.a`.
//!
//! This crate re-exports all `#[no_mangle] extern "C"` symbols from
//! `hadron-libc-core` and provides additional stub modules for unimplemented
//! POSIX interfaces. The staticlib crate type ensures all symbols end up
//! in the final `libc.a` archive.
#![no_std]

extern crate hadron_libc_core;

// Re-export all C ABI modules to ensure they're linked into the staticlib.
pub use hadron_libc_core::alloc;
pub use hadron_libc_core::atexit;
pub use hadron_libc_core::ctype;
pub use hadron_libc_core::dirent;
pub use hadron_libc_core::env;
pub use hadron_libc_core::errno;
pub use hadron_libc_core::flags;
pub use hadron_libc_core::io;
pub use hadron_libc_core::locale;
pub use hadron_libc_core::mman;
pub use hadron_libc_core::process;
pub use hadron_libc_core::signal;
pub use hadron_libc_core::start;
pub use hadron_libc_core::stdio;
pub use hadron_libc_core::string;
pub use hadron_libc_core::sys;
pub use hadron_libc_core::time;

// Stub modules for unimplemented POSIX interfaces.
// These return ENOSYS (-1) with correct signatures so that C code
// can link against them without undefined-symbol errors.
mod stubs;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
