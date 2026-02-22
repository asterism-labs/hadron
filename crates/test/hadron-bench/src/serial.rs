//! Serial port output for benchmark reporting.
//!
//! On x86_64: COM1 (port 0x3F8) via 8250 UART.
//! Provides both formatted text output and raw byte writing for binary data.

use core::fmt;

/// Initialize the serial port for benchmark output.
#[cfg(target_arch = "x86_64")]
pub fn init() {
    const COM1: u16 = 0x3F8;
    // SAFETY: Standard 8250 UART initialization sequence on COM1.
    // These I/O port writes configure the serial port for 38400 baud, 8N1.
    unsafe {
        outb(COM1 + 1, 0x00); // Disable interrupts
        outb(COM1 + 3, 0x80); // Enable DLAB
        outb(COM1 + 0, 0x03); // Baud divisor low (38400)
        outb(COM1 + 1, 0x00); // Baud divisor high
        outb(COM1 + 3, 0x03); // 8N1
        outb(COM1 + 2, 0xC7); // Enable FIFO, clear, 14-byte threshold
        outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR set
    }
}

/// Initialize the serial port (aarch64 stub).
#[cfg(target_arch = "aarch64")]
pub fn init() {
    todo!("aarch64 serial init")
}

/// Write a single raw byte to serial, waiting for TX ready.
///
/// Used for both formatted text output and binary wire format data.
pub fn write_byte(byte: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        const COM1: u16 = 0x3F8;
        // SAFETY: Polling the line status register and writing to the data
        // register is safe I/O port access for the 8250 UART.
        unsafe {
            while inb(COM1 + 5) & 0x20 == 0 {}
            outb(COM1, byte);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        let _ = byte;
        todo!("aarch64 serial write_byte")
    }
}

/// Write a slice of raw bytes to serial.
///
/// Used for binary wire format emission.
pub fn write_bytes(data: &[u8]) {
    for &byte in data {
        write_byte(byte);
    }
}

/// Serial writer implementing `fmt::Write`.
pub struct SerialWriter;

impl fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

/// Print to the serial port.
#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::serial::SerialWriter, $($arg)*);
    }};
}

/// Print to the serial port with a newline.
#[macro_export]
macro_rules! serial_println {
    () => { $crate::serial_print!("\n") };
    ($($arg:tt)*) => { $crate::serial_print!("{}\n", format_args!($($arg)*)) };
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn outb(port: u16, val: u8) {
    // SAFETY: Caller is responsible for ensuring the port is valid.
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") val,
            options(nomem, nostack, preserves_flags));
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: Caller is responsible for ensuring the port is valid.
    unsafe {
        core::arch::asm!("in al, dx", in("dx") port, out("al") val,
            options(nomem, nostack, preserves_flags));
    }
    val
}
