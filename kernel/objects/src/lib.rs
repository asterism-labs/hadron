//! Kernel object system for the Hadron microkernel.
//!
//! Every kernel resource is a reference-counted object (`Arc<dyn KernelObject>`)
//! accessed through handles in a per-process handle table. No raw object access
//! from userspace.
//!
//! This crate defines the object taxonomy, handle table, rights system, and all
//! kernel object types following a Zircon-style capability-based design.

#![no_std]

extern crate alloc;

pub mod handle;
pub mod object;
pub mod process;
pub mod thread;
pub mod vmar;
pub mod vmo;
