//! COM1 serial log sink.
//!
//! Registers a [`LogSink`] in the `hadron_log_sinks` linker section so that
//! Phase 1 log messages are written to the serial port. Phase 0 messages
//! already bypass sinks and write to COM1 directly via `hadron_log::early`.

use hadron_log::{FormattedRecord, Level, LogSink};

/// COM1 data register port.
const COM1_PORT: u16 = 0x3F8;

/// Writes a single byte to COM1.
#[inline]
fn serial_byte(b: u8) {
    // SAFETY: Port 0x3F8 is the standard COM1 data register.
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") COM1_PORT,
            in("al") b,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Writes a byte slice to COM1.
fn serial_bytes(bytes: &[u8]) {
    for &b in bytes {
        serial_byte(b);
    }
}

/// Writes a formatted log record to COM1.
///
/// Formats as `[LEVEL subsystem] message\n`. The level tag is right-padded
/// to 5 characters for alignment.
fn serial_write(record: &FormattedRecord<'_>) {
    serial_bytes(b"[");
    serial_bytes(level_tag(record.level).as_bytes());
    serial_bytes(b" ");
    serial_bytes(record.subsystem.as_bytes());
    serial_bytes(b"] ");
    serial_bytes(record.message.as_bytes());
    serial_byte(b'\n');
}

/// Returns a fixed-width tag string for the given log level.
fn level_tag(level: Level) -> &'static str {
    match level {
        Level::FATAL => "FATAL",
        Level::ERROR => "ERROR",
        Level::WARN => "WARN ",
        Level::INFO => "INFO ",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
        _ => "?????",
    }
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
