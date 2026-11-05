//! The Hadron kernel.
//!
//! It boots through Limine, sets up COM1 as the console, the GDT and the
//! IDT, switches to its own page tables, builds the physical memory map and
//! frame allocator, maps the heap, and exits QEMU through the `isa-debug-exit` device.
//!
//! Built with Gluon's `test` profile, it runs the kernel tests after boot
//! instead. See `ktest.rs`.
//!
//! The kernel builds for `x86_64-unknown-none` only. On a hosted target the
//! crate is an empty program, so `cargo test --workspace` and
//! `cargo clippy --workspace` still work on the development machine.

#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
extern crate alloc;

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod arch;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod boot;
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
mod panic;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod sync;

#[cfg(all(target_os = "none", not(target_arch = "x86_64")))]
compile_error!("the kernel supports only x86_64");

#[cfg(not(target_os = "none"))]
fn main() {}
