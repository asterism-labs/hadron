//! QEMU's `isa-debug-exit` device, which `quon.rs` adds to the machine.

use super::{cpu, port};

/// I/O port of the `isa-debug-exit` device, as set in `quon.rs`.
const DEBUG_EXIT_PORT: u16 = 0xf4;

/// Status the kernel reports when it leaves QEMU. QEMU exits with
/// `(value << 1) | 1`.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub(crate) enum ExitCode {
    /// QEMU exits with status 33, which `quon.rs` names as success.
    Success = 0x10,
    /// QEMU exits with status 35.
    Failure = 0x11,
}

/// Stops QEMU with `code`. Without the `isa-debug-exit` device, as on real
/// hardware, the write is ignored and the CPU halts.
pub(crate) fn exit(code: ExitCode) -> ! {
    // SAFETY: the port belongs to `isa-debug-exit`, and writing it ends the
    // QEMU process. On a machine without the device nothing listens there.
    unsafe { port::outl(DEBUG_EXIT_PORT, code as u32) };
    cpu::halt_forever()
}
