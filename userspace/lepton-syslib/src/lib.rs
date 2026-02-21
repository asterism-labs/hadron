//! Hadron userspace system library.
//!
//! Provides syscall wrappers, I/O primitives (`print!`/`println!`), a heap
//! allocator, and the `_start` entry point for userspace binaries running
//! on Hadron OS.

#![no_std]

extern crate alloc;

pub use hadron_syscall;

pub mod env;
pub mod heap;
pub mod io;
pub mod start;
pub mod sys;

#[global_allocator]
static HEAP: heap::UserHeap = heap::UserHeap::new();
