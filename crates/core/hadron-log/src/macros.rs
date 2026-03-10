//! Public logging macros.
//!
//! These macros provide the primary logging interface for kernel code.
//! Each macro performs compile-time level elimination, runtime level
//! filtering (one relaxed atomic load), and pushes a record into the
//! global MPSC ring buffer.
//!
//! All macros use `core::fmt` at the call site to format into a fixed
//! 128-byte inline buffer. This keeps the interface simple and supports
//! any type implementing `Display` or `Debug`.

/// Core logging macro.
///
/// Formats the message at the call site using `core::fmt`, then pushes
/// the record into the global MPSC ring buffer.
///
/// # Usage
///
/// ```ignore
/// klog!(Level::INFO, "acpi", "found {} DRHD entries", count);
/// ```
///
/// The hot path is zero-overhead when the level exceeds the compile-time
/// maximum: the entire call is eliminated.
#[macro_export]
macro_rules! klog {
    ($level:expr, $sub:expr, $($arg:tt)*) => {{
        // Compile-time elimination: if the level exceeds the static max,
        // this entire block is optimized away.
        const __LEVEL_VAL: u8 = ($level).0;
        const __MAX_VAL: u8 = $crate::__hadron_log_max_level!();
        if const { __LEVEL_VAL <= __MAX_VAL } {
            // Runtime filter: one relaxed atomic load.
            if __LEVEL_VAL <= $crate::runtime_level().load(::core::sync::atomic::Ordering::Relaxed) {
                let mut __buf = [0u8; 128];
                let __len = $crate::__format_into_buf(&mut __buf, format_args!($($arg)*));
                let __msg = $crate::RecordMessage::Formatted { buf: __buf, len: __len };
                $crate::__emit_log($level, $sub, __msg, file!(), line!());
            }
        }
    }};
}

/// Fatal-level log (severity 0).
#[macro_export]
macro_rules! kfatal {
    ($sub:expr, $($arg:tt)*) => { $crate::klog!($crate::Level::FATAL, $sub, $($arg)*) };
}

/// Error-level log (severity 10).
#[macro_export]
macro_rules! kerror {
    ($sub:expr, $($arg:tt)*) => { $crate::klog!($crate::Level::ERROR, $sub, $($arg)*) };
}

/// Warning-level log (severity 30).
#[macro_export]
macro_rules! kwarn {
    ($sub:expr, $($arg:tt)*) => { $crate::klog!($crate::Level::WARN, $sub, $($arg)*) };
}

/// Info-level log (severity 50).
#[macro_export]
macro_rules! kinfo {
    ($sub:expr, $($arg:tt)*) => { $crate::klog!($crate::Level::INFO, $sub, $($arg)*) };
}

/// Debug-level log (severity 100).
#[macro_export]
macro_rules! kdebug {
    ($sub:expr, $($arg:tt)*) => { $crate::klog!($crate::Level::DEBUG, $sub, $($arg)*) };
}

/// Trace-level log (severity 200).
#[macro_export]
macro_rules! ktrace {
    ($sub:expr, $($arg:tt)*) => { $crate::klog!($crate::Level::TRACE, $sub, $($arg)*) };
}

/// Enter a named span on the current CPU's span stack.
///
/// Returns an RAII guard that pops the span on drop.
///
/// # Usage
///
/// ```ignore
/// let _guard = kspan!("handling_irq");
/// ```
#[macro_export]
macro_rules! kspan {
    ($label:expr) => {
        $crate::enter_span($label)
    };
}
