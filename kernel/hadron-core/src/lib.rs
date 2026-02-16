//! Core library for Hadron OS, providing essential types and traits for kernel development.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]
#![cfg_attr(not(test), feature(custom_test_frameworks))]
#![feature(allocator_api, negative_impls, never_type)]

pub mod addr;
pub mod arch;
pub mod cell;
pub mod log;
pub mod mm;
pub mod paging;
pub mod percpu;
pub mod static_assert;
pub mod sync;
pub mod syscall;
pub mod task;
