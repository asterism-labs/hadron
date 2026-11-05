//! Limine entry point, COM1 output and QEMU exit for `x86_64`.

use core::arch::asm;
use core::fmt::{self, Write};
use core::panic::PanicInfo;

use limine::{BaseRevision, BootloaderInfoRequest, RequestsEndMarker, RequestsStartMarker};

#[used]
#[unsafe(link_section = ".limine_requests_start")]
static REQUESTS_START: RequestsStartMarker = RequestsStartMarker::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static BASE_REVISION: BaseRevision = BaseRevision::new();

#[used]
#[unsafe(link_section = ".limine_requests")]
static BOOTLOADER_INFO: BootloaderInfoRequest = BootloaderInfoRequest::new();

#[used]
#[unsafe(link_section = ".limine_requests_end")]
static REQUESTS_END: RequestsEndMarker = RequestsEndMarker::new();

/// I/O port of the first serial port's data register.
const COM1: u16 = 0x3f8;
/// I/O port of QEMU's `isa-debug-exit` device, as set in `gluon.rs`.
const DEBUG_EXIT: u16 = 0xf4;
/// Written to `DEBUG_EXIT` on success. QEMU exits with `(value << 1) | 1`,
/// so this becomes status 33, which `gluon.rs` names as success.
const EXIT_SUCCESS: u32 = 0x10;
/// Written to `DEBUG_EXIT` on failure. QEMU exits with status 35.
const EXIT_FAILURE: u32 = 0x11;

/// Writes to COM1. QEMU's UART transmits without any setup.
struct Serial;

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            // SAFETY: writing COM1's data register sends one byte and
            // touches no memory.
            unsafe { asm!("out dx, al", in("dx") COM1, in("al") byte, options(nomem, nostack)) };
        }
        Ok(())
    }
}

/// Stops QEMU with `code`, or halts forever where `isa-debug-exit` is absent.
fn exit_qemu(code: u32) -> ! {
    // SAFETY: the `isa-debug-exit` port ends the QEMU process. On a machine
    // without the device the write is ignored.
    unsafe { asm!("out dx, eax", in("dx") DEBUG_EXIT, in("eax") code, options(nomem, nostack)) };
    loop {
        // SAFETY: `hlt` only stops the CPU until the next interrupt.
        unsafe { asm!("hlt", options(nomem, nostack)) };
    }
}

/// Entry point. Limine jumps here, as named by `ENTRY` in `kernel.ld`.
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    if !BASE_REVISION.is_supported() {
        let _ = writeln!(
            Serial,
            "hadron: Limine does not support base revision {}",
            BASE_REVISION.revision()
        );
        exit_qemu(EXIT_FAILURE);
    }
    let (name, version) = BOOTLOADER_INFO
        .response()
        .map_or(("an unknown bootloader", ""), |info| {
            (info.name(), info.version())
        });
    let _ = writeln!(Serial, "hadron: booted by {name} {version}");
    exit_qemu(EXIT_SUCCESS)
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let _ = writeln!(Serial, "hadron: {info}");
    exit_qemu(EXIT_FAILURE)
}
