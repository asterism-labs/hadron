//! The kernel's panic handling. It prints the panic, then stops the kernel.
//!
//! The binary's `#[panic_handler]` calls [`handle`].
//!
//! Under QEMU the kernel exits with a failure status, so `gluon run` and
//! the test runner see the panic. On real hardware the CPU halts.

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86_64::{cpu, qemu};
use crate::console::kprintln;

/// Set by the first panic. A second panic, from code the first one runs,
/// skips straight to the exit.
static PANICKING: AtomicBool = AtomicBool::new(false);

/// Reports a panic on the console and stops the kernel.
pub fn handle(info: &PanicInfo<'_>) -> ! {
    cpu::disable_interrupts();
    if PANICKING.swap(true, Ordering::Relaxed) {
        kprintln!("hadron: panicked while panicking");
        qemu::exit(qemu::ExitCode::Failure);
    }
    crate::ktest::report_panic();
    match info.location() {
        Some(location) => kprintln!("hadron: kernel panic at {location}:"),
        None => kprintln!("hadron: kernel panic"),
    }
    kprintln!("{}", info.message());
    qemu::exit(qemu::ExitCode::Failure)
}
