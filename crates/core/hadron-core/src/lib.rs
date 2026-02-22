//! Core types and synchronization primitives for the Hadron kernel.
//!
//! This crate contains host-testable abstractions extracted from
//! `hadron-kernel`: address types, page/frame abstractions, task
//! metadata, and all synchronization primitives (spin locks, mutexes,
//! reader-writer locks, wait queues, and lockdep).
//!
//! By living outside the kernel crate, these types can be tested with
//! `cargo test`, loom, and miri on the host without a kernel target.

#![cfg_attr(not(test), no_std)]
#![feature(negative_impls)]
#![warn(missing_docs)]

extern crate alloc;

pub mod addr;
pub mod cell;
pub mod cpu_local;
pub mod id;
pub mod paging;
pub mod safety;
pub mod static_assert;
pub mod sched;
pub mod sync;
pub mod task;
