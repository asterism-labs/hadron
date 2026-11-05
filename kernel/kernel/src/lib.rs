//! The Hadron kernel.
//!
//! It boots through Limine, sets up the boot CPU's per-CPU block, COM3 as
//! the console, the GDT and the IDT, switches to its own page tables,
//! builds the physical memory map and frame allocator, and maps the heap.
//! The boot context then becomes the idle thread and starts the main
//! kernel thread, which exits QEMU through the `isa-debug-exit` device.
//!
//! Built with Gluon's `test` profile, it runs the kernel tests after boot
//! instead. See `ktest.rs`.
//!
//! The binary in `main.rs` holds only the entry point Limine jumps to and
//! the `#[panic_handler]`. Everything else lives in this library, so
//! other boot images can link it with their own entry point.
//!
//! The kernel builds for `x86_64-unknown-none` only. On a hosted target the
//! library is empty, so `cargo test --workspace` and
//! `cargo clippy --workspace` still work on the development machine.

#![cfg_attr(target_os = "none", no_std)]

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
extern crate alloc;

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod arch;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub mod boot;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod console;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod drivers;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod heap;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod ktest;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod memory;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub mod panic;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod percpu;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod sched;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod sync;

/// The kernel binary's main thread: runs the kernel tests under Gluon's
/// `test` profile, else returns, and QEMU exits with success.
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub fn kmain() {
    if hadron_config::KTEST {
        ktest::run();
    }
}

#[cfg(all(target_os = "none", not(target_arch = "x86_64")))]
compile_error!("the kernel supports only x86_64");
