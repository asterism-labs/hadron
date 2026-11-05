//! Kernel console on COM1, and the [`kprint!`] and [`kprintln!`] macros.
//!
//! The console takes no lock. One CPU runs the kernel and no interrupt
//! handler prints, so writes cannot interleave yet. SMP and interrupt
//! handlers will need the locking model first.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::drivers::uart16550::{InitError, Uart16550};

/// I/O port base of the first serial port on PCs.
const COM1_BASE: u16 = 0x3f8;
/// Baud rate of the console. QEMU ignores it.
const BAUD: u32 = 115_200;

// SAFETY: on PCs, ports 0x3f8 to 0x3ff belong to COM1, and only this module
// touches them.
static COM1: Uart16550 = unsafe { Uart16550::new(COM1_BASE) };

/// Set once COM1 passed its loopback check. Until then the console drops
/// output.
static READY: AtomicBool = AtomicBool::new(false);

/// Sets up COM1. If it fails, the console stays silent.
pub(crate) fn init() -> Result<(), InitError> {
    COM1.init(BAUD)?;
    READY.store(true, Ordering::Release);
    Ok(())
}

/// Reports whether [`init`] succeeded.
pub(crate) fn is_ready() -> bool {
    READY.load(Ordering::Acquire)
}

/// Writer for the console. Turns `\n` into `\r\n` for serial terminals.
pub(crate) struct Console;

impl Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if !is_ready() {
            return Ok(());
        }
        for byte in s.bytes() {
            if byte == b'\n' {
                COM1.write_byte(b'\r');
            }
            COM1.write_byte(byte);
        }
        Ok(())
    }
}

#[doc(hidden)]
pub(crate) fn print(args: fmt::Arguments<'_>) {
    // `Console::write_str` never fails.
    let _ = Console.write_fmt(args);
}

/// Prints to the kernel console.
macro_rules! kprint {
    ($($arg:tt)*) => {
        $crate::console::print(format_args!($($arg)*))
    };
}

/// Prints to the kernel console, followed by a newline.
macro_rules! kprintln {
    () => {
        $crate::console::kprint!("\n")
    };
    ($($arg:tt)*) => {
        $crate::console::print(format_args!("{}\n", format_args!($($arg)*)))
    };
}

pub(crate) use {kprint, kprintln};

ktest::ktest! {
    fn com1_passed_loopback_check() {
        assert!(is_ready());
    }
}
