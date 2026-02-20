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
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::drivers::early_console::{COM1, EarlySerial};
use crate::drivers::early_fb::EarlyFramebuffer;
use crate::sync::SpinLock;

// ---------------------------------------------------------------------------
// Log levels — lower = more severe
// ---------------------------------------------------------------------------

/// Kernel log severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum LogLevel {
    /// Fatal: unrecoverable error, system will halt.
    Fatal = 0,
    /// Error: something failed but the system may continue.
    Error = 1,
    /// Warning: unexpected condition, not necessarily an error.
    Warn = 2,
    /// Informational: high-level progress messages.
    Info = 3,
    /// Debug: detailed diagnostic information.
    Debug = 4,
    /// Trace: very verbose, low-level tracing.
    Trace = 5,
}

impl LogLevel {
    /// Returns the human-readable name (fixed-width for aligned output).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fatal => "FATAL",
            Self::Error => "ERROR",
            Self::Warn => "WARN ",
            Self::Info => "INFO ",
            Self::Debug => "DEBUG",
            Self::Trace => "TRACE",
        }
    }
}

// ---------------------------------------------------------------------------
// Raw print function (kprint! / kprintln!) — no levels, no filtering
// ---------------------------------------------------------------------------

/// The signature of the global print function.
pub type PrintFn = fn(fmt::Arguments<'_>);

fn null_print(_args: fmt::Arguments<'_>) {}

static PRINT_FN: AtomicPtr<()> = AtomicPtr::new(null_print as *mut ());

/// Registers the global print function.
///
/// # Safety
///
/// The provided function must be safe to call from any context. May be called
/// more than once (e.g., once for early serial, once for the full logger).
/// Uses `Release` ordering so subsequent loads see the new function.
pub unsafe fn set_print_fn(f: PrintFn) {
    PRINT_FN.store(f as *mut (), Ordering::Release);
}

/// Loads the current print function from the atomic pointer.
///
/// # Safety
///
/// Relies on the invariant that only valid `PrintFn` pointers (or the
/// initial `null_print`) are ever stored into `PRINT_FN`.
#[inline]
fn load_print_fn() -> PrintFn {
    let ptr = PRINT_FN.load(Ordering::Acquire);
    // SAFETY: We only ever store valid `PrintFn` function pointers into PRINT_FN.
    unsafe { core::mem::transmute(ptr) }
}

/// Implementation detail for [`kprint!`] / [`kprintln!`]. Not public API.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    load_print_fn()(args);
}

/// Prints to the kernel log sinks (raw, no level, no timestamp).
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => { $crate::log::_print(format_args!($($arg)*)) };
}

/// Prints to the kernel log sinks with a trailing newline (raw, no level).
#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => { $crate::kprint!("{}\n", format_args!($($arg)*)) };
}

// ---------------------------------------------------------------------------
// Leveled log function (klog! and convenience macros)
// ---------------------------------------------------------------------------

/// The signature of the global leveled log function.
pub type LogFn = fn(LogLevel, fmt::Arguments<'_>);

fn null_log(_level: LogLevel, _args: fmt::Arguments<'_>) {}

static LOG_FN: AtomicPtr<()> = AtomicPtr::new(null_log as *mut ());

/// Registers the global leveled log function.
///
/// # Safety
///
/// The provided function must be safe to call from any context. May be called
/// more than once (e.g., once for early serial, once for the full logger).
/// Uses `Release` ordering so subsequent loads see the new function.
pub unsafe fn set_log_fn(f: LogFn) {
    LOG_FN.store(f as *mut (), Ordering::Release);
}

/// Loads the current log function from the atomic pointer.
///
/// # Safety
///
/// Same invariant as [`load_print_fn`] — only valid `LogFn` pointers are stored.
#[inline]
fn load_log_fn() -> LogFn {
    let ptr = LOG_FN.load(Ordering::Acquire);
    // SAFETY: We only ever store valid `LogFn` function pointers into LOG_FN.
    unsafe { core::mem::transmute(ptr) }
}

/// Implementation detail for [`klog!`]. Not public API.
#[doc(hidden)]
pub fn _log(level: LogLevel, args: fmt::Arguments<'_>) {
    load_log_fn()(level, args);
}

/// Logs a message at the given level.
#[macro_export]
macro_rules! klog {
    ($level:expr, $($arg:tt)*) => {
        $crate::log::_log($level, format_args!($($arg)*))
    };
}

/// Logs a fatal-level message (level 0).
#[macro_export]
macro_rules! kfatal {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Fatal, $($arg)*) };
}

/// Logs an error-level message (level 1).
#[macro_export]
macro_rules! kerr {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Error, $($arg)*) };
}

/// Logs a warning-level message (level 2).
#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Warn, $($arg)*) };
}

/// Logs an info-level message (level 3).
#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Info, $($arg)*) };
}

/// Logs a debug-level message (level 4).
#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Debug, $($arg)*) };
}

/// Logs a trace-level message (level 5).
#[macro_export]
macro_rules! ktrace {
    ($($arg:tt)*) => { $crate::klog!($crate::log::LogLevel::Trace, $($arg)*) };
}

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
    fn max_level(&self) -> LogLevel;
    /// Human-readable name for diagnostics.
    fn name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// SerialSink
// ---------------------------------------------------------------------------

/// A [`LogSink`] that writes to a serial port via [`EarlySerial`].
pub struct SerialSink {
    serial: EarlySerial,
    max_level: LogLevel,
}

impl SerialSink {
    /// Creates a new serial sink.
    pub fn new(serial: EarlySerial, max_level: LogLevel) -> Self {
        Self { serial, max_level }
    }
}

impl LogSink for SerialSink {
    fn write_str(&self, s: &str) {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.serial.write_byte(b'\r');
            }
            self.serial.write_byte(byte);
        }
    }

    fn max_level(&self) -> LogLevel {
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
    max_level: LogLevel,
}

impl FramebufferSink {
    /// Creates a new framebuffer sink.
    pub fn new(fb: EarlyFramebuffer, max_level: LogLevel) -> Self {
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

    fn max_level(&self) -> LogLevel {
        self.max_level
    }

    fn name(&self) -> &str {
        "framebuffer"
    }
}

// ---------------------------------------------------------------------------
// DeviceFramebufferSink
// ---------------------------------------------------------------------------

/// A [`LogSink`] that writes to a framebuffer device (e.g., Bochs VGA)
/// via the device registry's `Arc<dyn Framebuffer>`.
pub struct DeviceFramebufferSink {
    fb: alloc::sync::Arc<dyn crate::driver_api::Framebuffer>,
    max_level: LogLevel,
}

impl DeviceFramebufferSink {
    /// Creates a new device framebuffer sink.
    pub fn new(
        fb: alloc::sync::Arc<dyn crate::driver_api::Framebuffer>,
        max_level: LogLevel,
    ) -> Self {
        Self { fb, max_level }
    }
}

impl LogSink for DeviceFramebufferSink {
    fn write_str(&self, s: &str) {
        use crate::drivers::early_fb::{CURSOR, GLYPH_HEIGHT, GLYPH_WIDTH};

        let info = self.fb.info();
        let cols = info.width / GLYPH_WIDTH;
        let rows = info.height / GLYPH_HEIGHT;

        let mut cursor = CURSOR.lock();
        for byte in s.bytes() {
            write_byte_to_fb(self.fb.as_ref(), &info, cols, rows, byte, &mut cursor);
        }
    }

    fn max_level(&self) -> LogLevel {
        self.max_level
    }

    fn name(&self) -> &str {
        "framebuffer"
    }
}

/// Renders a single byte onto a [`Framebuffer`](crate::driver_api::Framebuffer)
/// using the console font.
fn write_byte_to_fb(
    fb: &dyn crate::driver_api::Framebuffer,
    info: &crate::driver_api::framebuffer::FramebufferInfo,
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
        b'\x08' => {
            // Backspace: move cursor back one position (does not erase).
            // Erasure is done by the caller writing a space then another backspace.
            if cursor.col > 0 {
                cursor.col -= 1;
            } else if cursor.row > 0 {
                cursor.row -= 1;
                cursor.col = cols - 1;
            }
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
            draw_glyph_fb(fb, cursor.col, cursor.row, ch, FG_COLOR, BG_COLOR);
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
    fb: &dyn crate::driver_api::Framebuffer,
    col: u32,
    row: u32,
    ch: u8,
    fg: u32,
    bg: u32,
) {
    use crate::drivers::early_fb::{GLYPH_HEIGHT, GLYPH_WIDTH};
    use crate::drivers::font_console::px16;
    let offset = px16::glyph_index(ch as char)
        .or_else(|| px16::glyph_index(' '))
        .unwrap_or(0);
    let glyph = &px16::DATA[offset..][..px16::BYTES_PER_GLYPH];
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
    fb: &dyn crate::driver_api::Framebuffer,
    info: &crate::driver_api::framebuffer::FramebufferInfo,
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

/// Wrapper around [`EarlySerial`] that implements `fmt::Write`.
struct SerialWriter(EarlySerial);

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
    let mut w = SerialWriter(EarlySerial::new(COM1));
    let _ = w.write_fmt(args);
}

/// Early log function: formats a leveled, timestamped message to COM1.
fn early_serial_log(level: LogLevel, args: fmt::Arguments<'_>) {
    let nanos = crate::time::boot_nanos();
    let total_micros = nanos / 1_000;
    let secs = total_micros / 1_000_000;
    let micros = total_micros % 1_000_000;
    let level_str = level.name();

    let mut w = SerialWriter(EarlySerial::new(COM1));
    let _ = write!(w, "[{secs:>5}.{micros:06}] {level_str} {args}\n");
}

/// Registers early serial print/log functions.
///
/// Call this after UART hardware init and before any `kprint!`/`klog!` use.
/// No heap allocation required.
pub fn init_early_serial() {
    // SAFETY: Both functions are safe to call from any context — they
    // construct a Uart16550 on the stack (just a u16) and write bytes.
    unsafe {
        set_print_fn(early_serial_print);
        set_log_fn(early_serial_log);
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
                EarlySerial::new(COM1),
                LogLevel::Trace,
            ));
            let mut sinks: Vec<Box<dyn LogSink>> = Vec::with_capacity(4);
            sinks.push(serial_sink);
            *guard = Some(LoggerInner { sinks });
        }

        // Replace early serial functions with the logger's functions.
        // SAFETY: logger_print and logger_log are safe to call from any context.
        unsafe {
            set_print_fn(logger_print);
            set_log_fn(logger_log);
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
    fn log(&self, level: LogLevel, args: fmt::Arguments<'_>) {
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
fn logger_log(level: LogLevel, args: fmt::Arguments<'_>) {
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

/// Writes a panic message directly to COM1 via [`EarlySerial`].
///
/// No locks, no allocation — safe from any context including inside a
/// panic while the logger lock is held.
pub fn panic_serial(info: &core::panic::PanicInfo) {
    let mut w = SerialWriter(EarlySerial::new(COM1));
    let _ = write!(w, "\n!!! KERNEL PANIC !!!\n{info}\n");
    crate::backtrace::panic_backtrace(&mut w);
}
