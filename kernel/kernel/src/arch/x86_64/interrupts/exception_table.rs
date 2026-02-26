//! Declarative macro for type-safe IDT exception handler registration.
//!
//! The [`exception_table!`] macro enforces at compile time that each exception
//! handler has the correct signature (plain, with error code, diverging, etc.).
//! A non-diverging Double Fault handler, for example, will fail to compile with
//! a clear type mismatch error.

/// Registers exception handlers into an IDT with compile-time signature
/// enforcement.
///
/// Each entry specifies the IDT field name, handler kind, and handler path.
/// The macro verifies the handler's type against the appropriate type alias
/// before delegating to the corresponding `IdtEntry` method.
///
/// # Handler kinds
///
/// | Kind | IDT method | Expected signature |
/// |------|------------|--------------------|
/// | `plain` | `set_handler` | `extern "x86-interrupt" fn(InterruptStackFrame)` |
/// | `with_err_code` | `set_handler_with_err_code` | `extern "x86-interrupt" fn(InterruptStackFrame, u64)` |
/// | `diverging` | `set_diverging_handler` | `extern "x86-interrupt" fn(InterruptStackFrame) -> !` |
/// | `diverging_err` | `set_diverging_handler_with_err_code` | `extern "x86-interrupt" fn(InterruptStackFrame, u64) -> !` |
///
/// # Optional attributes
///
/// - `ist = N` — sets the IST index on the entry (0-7)
/// - `dpl = N` — sets the descriptor privilege level (0-3)
///
/// # Example
///
/// ```ignore
/// exception_table! {
///     idt = idt;
///     divide_error => plain(handlers::divide_error);
///     breakpoint   => plain(handlers::breakpoint), dpl = 3;
///     double_fault => diverging_err(handlers::double_fault), ist = 1;
///     page_fault   => with_err_code(handlers::page_fault);
/// }
/// ```
macro_rules! exception_table {
    // Entry point: iterate over all entries.
    (
        idt = $idt:ident;
        $($name:ident => $kind:ident($handler:path) $(, ist = $ist:expr)? $(, dpl = $dpl:expr)?;)*
    ) => {
        $(
            exception_table!(@register $idt, $name, $kind, $handler $(, ist = $ist)? $(, dpl = $dpl)?);
        )*
    };

    // --- @register arms: type-check then delegate to the IDT method ---

    (@register $idt:ident, $name:ident, plain, $handler:path $(, ist = $ist:expr)? $(, dpl = $dpl:expr)?) => {
        {
            // Compile-time signature enforcement.
            let _: $crate::arch::x86_64::structures::idt::HandlerFunc = $handler;
            let _opts = $idt.$name.set_handler($handler);
            $(_opts.set_ist_index($ist);)?
            $(_opts.set_dpl($dpl);)?
        }
    };

    (@register $idt:ident, $name:ident, with_err_code, $handler:path $(, ist = $ist:expr)? $(, dpl = $dpl:expr)?) => {
        {
            let _: $crate::arch::x86_64::structures::idt::HandlerFuncWithErrCode = $handler;
            let _opts = $idt.$name.set_handler_with_err_code($handler);
            $(_opts.set_ist_index($ist);)?
            $(_opts.set_dpl($dpl);)?
        }
    };

    (@register $idt:ident, $name:ident, diverging, $handler:path $(, ist = $ist:expr)? $(, dpl = $dpl:expr)?) => {
        {
            let _: $crate::arch::x86_64::structures::idt::DivergingHandlerFunc = $handler;
            let _opts = $idt.$name.set_diverging_handler($handler);
            $(_opts.set_ist_index($ist);)?
            $(_opts.set_dpl($dpl);)?
        }
    };

    (@register $idt:ident, $name:ident, diverging_err, $handler:path $(, ist = $ist:expr)? $(, dpl = $dpl:expr)?) => {
        {
            let _: $crate::arch::x86_64::structures::idt::DivergingHandlerFuncWithErrCode = $handler;
            let _opts = $idt.$name.set_diverging_handler_with_err_code($handler);
            $(_opts.set_ist_index($ist);)?
            $(_opts.set_dpl($dpl);)?
        }
    };
}

pub(crate) use exception_table;
