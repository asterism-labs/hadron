//! Kernel tests using the `#[kernel_test]` staged test framework.
//!
//! Compiled only when `--cfg ktest` is active. Tests are organized by
//! subsystem and placed into the appropriate boot stage.

extern crate alloc;

mod alt_fn;
#[cfg(hadron_alt_instructions)]
mod alt_instr;
mod boot;
mod heap;
mod pmm;
mod backtrace;
mod trace;
mod syscall;
mod profiling;
mod vmm;
mod vfs;
mod sched;
mod pci;
mod proc;
