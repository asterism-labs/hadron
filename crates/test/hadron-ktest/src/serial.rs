//! Minimal serial output for kernel test reporting.
//!
//! On x86_64: writes directly to the 8250 UART on COM1 (port 0x3F8).
//! On aarch64: stub (todo).
//!
//! This works at any point after CPU initialization, including before the
//! kernel's full logging infrastructure is available.

use core::fmt::{self, Write};

/// Serial writer implementing `fmt::Write`.
pub struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &byte in s.as_bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

#[cfg(target_arch = "x86_64")]
fn write_byte(byte: u8) {
    const COM1: u16 = 0x3F8;
    // SAFETY: Port I/O to COM1 is safe after CPU init in the kernel.
    unsafe {
        while inb(COM1 + 5) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        outb(COM1, byte);
    }
}

#[cfg(target_arch = "aarch64")]
fn write_byte(_byte: u8) {
    todo!("aarch64 serial write_byte")
}

/// Writes formatted output to the serial port.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    SerialWriter.write_fmt(args).unwrap();
}

/// Prints to the serial console (COM1).
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::serial::SerialWriter, $($arg)*);
    }};
}

/// Prints to the serial console (COM1), with a newline.
#[macro_export]
macro_rules! serial_println {
    () => { $crate::serial_print!("\n") };
    ($($arg:tt)*) => { $crate::serial_print!("{}\n", format_args!($($arg)*)) };
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn outb(port: u16, val: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") val,
            options(nomem, nostack, preserves_flags));
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe {
        core::arch::asm!("in al, dx", in("dx") port, out("al") val,
            options(nomem, nostack, preserves_flags));
    }
    val
}
