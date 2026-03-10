//! COM1 serial log sink.
//!
//! Registers a [`LogSink`] in the `hadron_log_sinks` linker section so that
//! log messages are written to the serial port. Outputs the pre-formatted
//! `formatted_line` from each record, ensuring identical formatting across
//! all log paths.

use hadron_log::{FormattedRecord, Level, LogSink};

use crate::arch::x86_64::Port;

/// COM1 serial port (data register at `0x3F8`).
const COM1: Port<u8> = Port::new(0x3F8);

/// Writes a byte slice to COM1.
fn serial_bytes(bytes: &[u8]) {
    for &b in bytes {
        // SAFETY: Port 0x3F8 is the standard COM1 data register.
        // Writing bytes to it produces serial output with no side effects
        // beyond character transmission.
        unsafe { COM1.write(b) };
    }
}

/// Writes a pre-formatted log line to COM1.
fn serial_write(record: &FormattedRecord<'_>) {
    serial_bytes(record.formatted_line);
}

/// Flush callback — no-op since COM1 is unbuffered.
fn serial_flush() {}

hadron_linkset::linkset_entry!("hadron_log_sinks",
    SERIAL_SINK: LogSink = LogSink {
        name: "serial",
        write: serial_write,
        flush: serial_flush,
        min_level: Level::ALL,
    }
);
