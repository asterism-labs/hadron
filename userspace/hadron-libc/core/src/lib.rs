//! Core implementation of Hadron's POSIX-compatible C library.
//!
//! This crate contains the idiomatic Rust implementations of libc functions.
//! It is compiled as an rlib and consumed by `hadron-libc` (the staticlib shell)
//! which exports the C ABI symbols into `libc.a`.
#![no_std]
#![allow(internal_features)]
#![feature(lang_items)]
#![feature(c_variadic)]
#![feature(naked_functions)]

#[cfg(feature = "userspace")]
pub mod alloc;
pub mod atexit;
pub mod ctype;
#[cfg(feature = "userspace")]
pub mod dirent;
#[cfg(feature = "userspace")]
pub mod env;
pub mod errno;
pub mod flags;
#[cfg(feature = "userspace")]
pub mod io;
pub mod locale;
#[cfg(feature = "userspace")]
pub mod mman;
#[cfg(feature = "userspace")]
pub mod poll;
#[cfg(feature = "userspace")]
pub mod process;
#[cfg(feature = "userspace")]
pub mod pthread;
#[cfg(feature = "userspace")]
pub mod signal;
#[cfg(feature = "userspace")]
pub mod socket;
#[cfg(feature = "userspace")]
pub mod start;
#[cfg(feature = "userspace")]
pub mod stdio;
pub mod string;
#[cfg(feature = "userspace")]
pub mod sys;
#[cfg(feature = "userspace")]
pub mod time;

#[cfg(not(test))]
#[lang = "eh_personality"]
fn eh_personality() {}
