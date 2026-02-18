//! Native Hadron syscall number constants and user-space pointer validation.
//!
//! Syscall numbers, error codes, data structures, and introspection enums are
//! defined in the `hadron-syscall` crate and re-exported here for backward
//! compatibility. Kernel-specific pointer validation lives in [`userptr`].

pub mod userptr;

pub use hadron_syscall::*;
