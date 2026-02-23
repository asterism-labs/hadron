//! Kernel tests using the `#[kernel_test]` staged test framework.
//!
//! Compiled only when `--cfg ktest` is active. Tests are organized by
//! subsystem and placed into the appropriate boot stage.

extern crate alloc;

mod alt_fn;
#[cfg(hadron_alt_instructions)]
mod alt_instr;
mod backtrace;
mod boot;
mod heap;
mod pci;
mod pmm;
mod proc;
mod profiling;
mod sched;
mod syscall;
mod trace;
mod vfs;
mod vmm;
