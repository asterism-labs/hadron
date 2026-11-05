//! Placeholder kernel. Replace this crate when the real kernel work starts.
//!
//! It boots through Limine, writes one line to COM1 and exits QEMU through
//! the `isa-debug-exit` device, which is enough for `gluon run` to show
//! that the build and boot path works.
//!
//! Built with Gluon's `test` profile, it runs the kernel tests after boot
//! instead. See `ktest.rs`.
//!
//! The kernel builds for `x86_64-unknown-none` only. On a hosted target the
//! crate is an empty program, so `cargo test --workspace` and
//! `cargo clippy --workspace` still work on the development machine.

#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod boot;
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod ktest;

#[cfg(all(target_os = "none", not(target_arch = "x86_64")))]
compile_error!("the placeholder kernel supports only x86_64");

#[cfg(not(target_os = "none"))]
fn main() {}
