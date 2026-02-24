//! Inter-process communication primitives.
//!
//! Provides channels for message-oriented IPC, pipes for byte-oriented IPC,
//! and futex for fast userspace mutexes.

pub mod channel;
pub(crate) mod circular_buffer;
pub mod futex;
pub mod pipe;
pub mod service;
pub mod shm;
