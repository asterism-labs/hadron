//! Kernel logging infrastructure.
//!
//! Provides a two-phase logging system:
//!
//! **Phase 1 — Early Serial (pre-heap):** [`init_early_serial`] registers
//! lightweight print/log functions that write directly to COM1 with no locks
//! and no allocation. All output during GDT, IDT, PMM, VMM, and heap init
//! goes through this path.
//!
//! **Phase 2 — Full Logger (post-heap):** [`init_logger`] creates a
//! [`Logger`] with a `Vec<Box<dyn LogSink>>` and replaces the early serial
//! functions. Additional sinks (e.g., framebuffer) are registered via
//! [`add_sink`].

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::fmt::{self, Write as _};

#[cfg(target_arch = "x86_64")]
use hadron_drivers::uart16550::{COM1, Uart16550};

use crate::drivers::early_fb::EarlyFramebuffer;
use crate::sync::SpinLock;

// ---------------------------------------------------------------------------
// LogSink trait
// ---------------------------------------------------------------------------

/// A dyn-compatible output sink for the kernel logger.
///
/// Uses `&self` (not `&mut self`) because:
/// - `Uart16550::write_byte` takes `&self` (port I/O is stateless)
/// - `EarlyFramebuffer` cursor is in a separate `SpinLock`
pub trait LogSink: Send + Sync {
    /// Write a string fragment to this sink.
    fn write_str(&self, s: &str);
    /// Maximum log level accepted (messages with `level <= max_level` are written).
    fn max_level(&self) -> hadron_core::log::LogLevel;
    /// Human-readable name for diagnostics.
    fn name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// SerialSink
// ---------------------------------------------------------------------------

/// A [`LogSink`] that writes to a 16550 UART serial port.
pub struct SerialSink {
    uart: Uart16550,
    max_level: hadron_core::log::LogLevel,
}

impl SerialSink {
    /// Creates a new serial sink.
    pub fn new(uart: Uart16550, max_level: hadron_core::log::LogLevel) -> Self {
        Self { uart, max_level }
    }
}

impl LogSink for SerialSink {
    fn write_str(&self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.uart.write_byte(b'\r');
            }
            self.uart.write_byte(byte);
        }
    }

    fn max_level(&self) -> hadron_core::log::LogLevel {
        self.max_level
    }

    fn name(&self) -> &str {
        "serial"
    }
}

// ---------------------------------------------------------------------------
// FramebufferSink
// ---------------------------------------------------------------------------

/// A [`LogSink`] that writes to the early framebuffer console.
pub struct FramebufferSink {
    fb: EarlyFramebuffer,
    max_level: hadron_core::log::LogLevel,
}

impl FramebufferSink {
    /// Creates a new framebuffer sink.
    pub fn new(fb: EarlyFramebuffer, max_level: hadron_core::log::LogLevel) -> Self {
        Self { fb, max_level }
    }
}

impl LogSink for FramebufferSink {
    fn write_str(&self, s: &str) {
        let mut cursor = crate::drivers::early_fb::CURSOR.lock();
        for byte in s.bytes() {
            self.fb.write_byte_internal(byte, &mut cursor);
        }
    }

    fn max_level(&self) -> hadron_core::log::LogLevel {
        self.max_level
    }

    fn name(&self) -> &str {
        "framebuffer"
    }
}

// ---------------------------------------------------------------------------
// BochsVgaSink
// ---------------------------------------------------------------------------

/// A [`LogSink`] that writes to the Bochs VGA framebuffer via the driver's
/// [`Framebuffer`] trait implementation.
pub struct BochsVgaSink {
    max_level: hadron_core::log::LogLevel,
}

impl BochsVgaSink {
    /// Creates a new Bochs VGA sink.
    pub fn new(max_level: hadron_core::log::LogLevel) -> Self {
        Self { max_level }
    }
}

impl LogSink for BochsVgaSink {
    fn write_str(&self, s: &str) {
        use crate::drivers::early_fb::{CURSOR, GLYPH_HEIGHT, GLYPH_WIDTH, VGA_FONT_8X16_REF};
        use hadron_driver_api::Framebuffer;

        hadron_drivers::bochs_vga::with_bochs_vga(|vga| {
            let info = vga.info();
            let cols = info.width / GLYPH_WIDTH;
            let rows = info.height / GLYPH_HEIGHT;
            let font = VGA_FONT_8X16_REF;

            let mut cursor = CURSOR.lock();
            for byte in s.bytes() {
                write_byte_to_fb(vga, &info, font, cols, rows, byte, &mut cursor);
            }
        });
    }

    fn max_level(&self) -> hadron_core::log::LogLevel {
        self.max_level
    }

    fn name(&self) -> &str {
        "framebuffer"
    }
}

/// Renders a single byte onto a [`Framebuffer`] using the VGA font.
fn write_byte_to_fb(
    fb: &dyn hadron_driver_api::Framebuffer,
    info: &hadron_driver_api::framebuffer::FramebufferInfo,
    font: &[u8],
    cols: u32,
    rows: u32,
    byte: u8,
    cursor: &mut crate::drivers::early_fb::CursorState,
) {
    /// Foreground color (light grey in BGR32).
    const FG_COLOR: u32 = 0x00_AA_AA_AA;
    /// Background color (black).
    const BG_COLOR: u32 = 0x00_00_00_00;

    match byte {
        b'\n' => {
            cursor.col = 0;
            cursor.row += 1;
        }
        b'\r' => {
            cursor.col = 0;
        }
        b'\t' => {
            let next = (cursor.col + 4) & !3;
            cursor.col = next;
            if cursor.col >= cols {
                cursor.col = 0;
                cursor.row += 1;
            }
        }
        ch => {
            if cursor.col >= cols {
                cursor.col = 0;
                cursor.row += 1;
            }
            if cursor.row >= rows {
                scroll_up_fb(fb, info, rows);
                cursor.row = rows - 1;
            }
            draw_glyph_fb(fb, font, cursor.col, cursor.row, ch, FG_COLOR, BG_COLOR);
            cursor.col += 1;
        }
    }

    if cursor.row >= rows {
        scroll_up_fb(fb, info, rows);
        cursor.row = rows - 1;
    }
}

/// Draws a single glyph at character position (col, row) onto a framebuffer.
fn draw_glyph_fb(
    fb: &dyn hadron_driver_api::Framebuffer,
    font: &[u8],
    col: u32,
    row: u32,
    ch: u8,
    fg: u32,
    bg: u32,
) {
    use crate::drivers::early_fb::{GLYPH_HEIGHT, GLYPH_WIDTH};
    let glyph = &font[(ch as usize) * (GLYPH_HEIGHT as usize)..][..(GLYPH_HEIGHT as usize)];
    let x0 = col * GLYPH_WIDTH;
    let y0 = row * GLYPH_HEIGHT;

    for (dy, &scanline) in glyph.iter().enumerate() {
        for dx in 0..GLYPH_WIDTH {
            let bit = (scanline >> (7 - dx)) & 1;
            let color = if bit != 0 { fg } else { bg };
            fb.put_pixel(x0 + dx, y0 + dy as u32, color);
        }
    }
}

/// Scrolls the framebuffer up by one glyph row.
fn scroll_up_fb(
    fb: &dyn hadron_driver_api::Framebuffer,
    info: &hadron_driver_api::framebuffer::FramebufferInfo,
    rows: u32,
) {
    use crate::drivers::early_fb::GLYPH_HEIGHT;
    if rows <= 1 {
        return;
    }
    let row_bytes = info.pitch as usize * GLYPH_HEIGHT as usize;
    let src_offset = row_bytes as u64;
    let copy_count = row_bytes * (rows as usize - 1);
    // SAFETY: Scroll copies within the valid framebuffer region.
    unsafe {
        fb.copy_within(src_offset, 0, copy_count);
        fb.fill_zero((row_bytes * (rows as usize - 1)) as u64, row_bytes);
    }
}

// ---------------------------------------------------------------------------
// Early serial functions (Phase 1, pre-heap)
// ---------------------------------------------------------------------------

/// Wrapper around `Uart16550` that implements `fmt::Write` using `&self`
/// semantics (constructs on the stack each time, no state).
struct SerialWriter(Uart16550);

impl fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.0.write_byte(b'\r');
            }
            self.0.write_byte(byte);
        }
        Ok(())
    }
}

/// Early print function: writes directly to COM1 with no locks.
fn early_serial_print(args: fmt::Arguments<'_>) {
    let mut w = SerialWriter(Uart16550::new(COM1));
    let _ = w.write_fmt(args);
}

/// Early log function: formats a leveled, timestamped message to COM1.
fn early_serial_log(level: hadron_core::log::LogLevel, args: fmt::Arguments<'_>) {
    let nanos = crate::time::boot_nanos();
    let total_micros = nanos / 1_000;
    let secs = total_micros / 1_000_000;
    let micros = total_micros % 1_000_000;
    let level_str = level.name();

    let mut w = SerialWriter(Uart16550::new(COM1));
    let _ = write!(w, "[{secs:>5}.{micros:06}] {level_str} {args}\n");
}

/// Registers early serial print/log functions with `hadron_core`.
///
/// Call this after UART hardware init and before any `kprint!`/`klog!` use.
/// No heap allocation required.
pub fn init_early_serial() {
    // SAFETY: Both functions are safe to call from any context — they
    // construct a Uart16550 on the stack (just a u16) and write bytes.
    unsafe {
        hadron_core::log::set_print_fn(early_serial_print);
        hadron_core::log::set_log_fn(early_serial_log);
    }
}

// ---------------------------------------------------------------------------
// Logger (Phase 2, post-heap)
// ---------------------------------------------------------------------------

/// Interior data protected by the logger's spin lock.
struct LoggerInner {
    sinks: Vec<Box<dyn LogSink>>,
}

/// The kernel logger.
///
/// Holds a `Vec<Box<dyn LogSink>>` behind a [`SpinLock`]. Output is fanned out
/// to every registered sink. Construct with [`Logger::new`] (const) and store
/// in a `static`.
pub struct Logger {
    inner: SpinLock<Option<LoggerInner>>,
}

impl Logger {
    /// Creates a new logger (uninitialized). Writes are silent no-ops until
    /// [`init_with_serial`](Self::init_with_serial) is called.
    const fn new() -> Self {
        Self {
            inner: SpinLock::new(None),
        }
    }

    /// Initializes the logger with a serial sink pre-registered, then replaces
    /// the early serial functions with the logger's functions. Zero-loss
    /// transition.
    fn init_with_serial(&self) {
        {
            let mut guard = self.inner.lock();
            let serial_sink = Box::new(SerialSink::new(
                Uart16550::new(COM1),
                hadron_core::log::LogLevel::Trace,
            ));
            let mut sinks: Vec<Box<dyn LogSink>> = Vec::with_capacity(4);
            sinks.push(serial_sink);
            *guard = Some(LoggerInner { sinks });
        }

        // Replace early serial functions with the logger's functions.
        // SAFETY: logger_print and logger_log are safe to call from any context.
        unsafe {
            hadron_core::log::set_print_fn(logger_print);
            hadron_core::log::set_log_fn(logger_log);
        }
    }

    /// Registers an additional output sink.
    fn add_sink(&self, sink: Box<dyn LogSink>) {
        let mut guard = self.inner.lock();
        if let Some(inner) = guard.as_mut() {
            inner.sinks.push(sink);
        }
    }

    /// Replaces the first sink whose [`name()`](LogSink::name) matches `name`
    /// with `new_sink`. Returns `true` if a replacement was made.
    fn replace_sink_by_name(&self, name: &str, new_sink: Box<dyn LogSink>) -> bool {
        let mut guard = self.inner.lock();
        if let Some(inner) = guard.as_mut() {
            for sink in &mut inner.sinks {
                if sink.name() == name {
                    *sink = new_sink;
                    return true;
                }
            }
        }
        false
    }

    /// Raw write — fans out `args` to **all** sinks with no filtering.
    /// Used by `kprint!` / `kprintln!` (panic handlers, raw console).
    fn write_fmt(&self, args: fmt::Arguments<'_>) {
        let guard = self.inner.lock();
        if let Some(inner) = guard.as_ref() {
            for sink in &inner.sinks {
                let mut w = SinkWriter(sink.as_ref());
                let _ = fmt::Write::write_fmt(&mut w, args);
            }
        }
    }

    /// Leveled write — formats a timestamped, level-tagged message and writes
    /// it only to sinks whose `max_level >= level`.
    fn log(&self, level: hadron_core::log::LogLevel, args: fmt::Arguments<'_>) {
        let nanos = crate::time::boot_nanos();
        let total_micros = nanos / 1_000;
        let secs = total_micros / 1_000_000;
        let micros = total_micros % 1_000_000;
        let level_str = level.name();

        let guard = self.inner.lock();
        if let Some(inner) = guard.as_ref() {
            for sink in &inner.sinks {
                if level <= sink.max_level() {
                    let mut w = SinkWriter(sink.as_ref());
                    let _ = write!(w, "[{secs:>5}.{micros:06}] {level_str} {args}\n");
                }
            }
        }
    }
}

/// Adapter that wraps a `&dyn LogSink` to implement `fmt::Write`.
struct SinkWriter<'a>(&'a dyn LogSink);

impl fmt::Write for SinkWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.write_str(s);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Global logger instance and public API
// ---------------------------------------------------------------------------

/// Global logger instance.
pub static LOGGER: Logger = Logger::new();

/// Print function that forwards to the global logger (raw, unfiltered).
fn logger_print(args: fmt::Arguments<'_>) {
    LOGGER.write_fmt(args);
}

/// Log function that forwards to the global logger (leveled, timestamped).
fn logger_log(level: hadron_core::log::LogLevel, args: fmt::Arguments<'_>) {
    LOGGER.log(level, args);
}

/// Initializes the full logger (Phase 2), replacing early serial functions.
///
/// Call this after the heap allocator is available.
pub fn init_logger() {
    LOGGER.init_with_serial();
}

/// Registers an additional output sink with the global logger.
pub fn add_sink(sink: Box<dyn LogSink>) {
    LOGGER.add_sink(sink);
}

/// Replaces a named sink in the global logger. Returns `true` on success.
pub fn replace_sink_by_name(name: &str, new_sink: Box<dyn LogSink>) -> bool {
    LOGGER.replace_sink_by_name(name, new_sink)
}

// ---------------------------------------------------------------------------
// Panic helper
// ---------------------------------------------------------------------------

/// Writes a panic message directly to COM1 via `Uart16550`.
///
/// No locks, no allocation — safe from any context including inside a
/// panic while the logger lock is held.
pub fn panic_serial(info: &core::panic::PanicInfo) {
    let mut w = SerialWriter(Uart16550::new(COM1));
    let _ = write!(w, "\n!!! KERNEL PANIC !!!\n{info}\n");
    crate::backtrace::panic_backtrace(&mut w);
}
