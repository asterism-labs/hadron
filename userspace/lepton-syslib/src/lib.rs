//! Hadron userspace system library.
//!
//! Provides syscall wrappers, I/O primitives (`print!`/`println!`), and the
//! `_start` entry point for userspace binaries running on Hadron OS.

#![no_std]

pub use hadron_syscall;

pub mod io;
pub mod start;
pub mod sys;
