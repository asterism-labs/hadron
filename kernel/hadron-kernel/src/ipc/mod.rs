//! Inter-process communication primitives.
//!
//! Provides pipes for byte-oriented IPC and futex for fast userspace mutexes.

pub mod futex;
pub mod pipe;
