//! Inter-process communication primitives.
//!
//! Pure IPC logic (pipes, channels, services) lives in the `hadron-ipc` crate.
//! This module re-exports those types and provides kernel-specific IPC
//! (futex, shared memory) that depends on kernel internals.

pub use hadron_ipc::channel;
pub use hadron_ipc::circular_buffer;
pub use hadron_ipc::pipe;
pub use hadron_ipc::service;

pub mod futex;
pub mod shm;
