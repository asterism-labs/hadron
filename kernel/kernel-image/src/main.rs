//! Thin binary wrapper that links `hadron-kernel` into a standalone ELF.
//!
//! The actual kernel entry point (`kernel_init`) lives in `hadron_kernel::entry`.
//! This crate exists solely to produce a linked ELF binary with the kernel
//! linker script applied. The resulting binary is embedded into the UEFI boot
//! stub at build time.

#![no_std]
#![no_main]

extern crate hadron_kernel;

use hadron_kernel::arch::x86_64::structures::machine_state::MachineState;

// Force the linker to retain `kernel_init` (the entry point declared in the
// linker script) even though nothing in *this* crate references it directly.
core::arch::global_asm!(".global kernel_init");

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut out = SerialWriter;

    // Header with location.
    serial_str("KERNEL PANIC: ");
    if let Some(loc) = info.location() {
        serial_str(loc.file());
        serial_str(":");
        let mut buf = [0u8; 10];
        let s = fmt_u32(loc.line(), &mut buf);
        serial_str(s);
    }
    serial_str("\n");

    // Print the panic message (contains CR2, error code for page faults).
    use core::fmt::Write;
    let _ = write!(out, "{}\n", info.message());

    // Machine state snapshot.
    let state = MachineState::capture();
    let _ = core::fmt::write(&mut out, format_args!("{state}\n"));

    loop {
        core::hint::spin_loop();
    }
}

/// Minimal `core::fmt::Write` implementation that writes to COM1.
struct SerialWriter;

impl core::fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        serial_str(s);
        Ok(())
    }
}

fn serial_str(s: &str) {
    for b in s.bytes() {
        // SAFETY: Port 0x3F8 is the standard COM1 data register.
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") 0x3F8u16,
                in("al") b,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

fn fmt_u32(mut n: u32, buf: &mut [u8; 10]) -> &str {
    if n == 0 {
        return "0";
    }
    let mut i = 10;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // SAFETY: digits are all ASCII.
    unsafe { core::str::from_utf8_unchecked(&buf[i..]) }
}
