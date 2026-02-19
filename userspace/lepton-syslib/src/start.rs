//! Entry point and panic handler for userspace binaries.
//!
//! Provides `_start` which reads argc/argv from the stack (placed by the kernel),
//! constructs a `&[&str]` argument slice, and calls the user-defined
//! `main(args: &[&str]) -> i32`.

/// Naked entry point: reads argc and argv base from the stack, then calls
/// `_start_rust`.
///
/// The kernel writes the following layout at RSP:
/// ```text
///   RSP + 0  → argc: usize
///   RSP + 8  → (ptr₀, len₀), (ptr₁, len₁), ...  ← &str pairs
/// ```
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub extern "C" fn _start() -> ! {
    // RSP points to argc. Argv (ptr,len) pairs start at RSP+8.
    // Load argc into RDI, argv pointer into RSI, call _start_rust.
    core::arch::naked_asm!(
        "mov rdi, [rsp]",
        "lea rsi, [rsp + 8]",
        "call {start_rust}",
        start_rust = sym _start_rust,
    );
}

/// Rust entry point called from the naked `_start`.
///
/// Constructs a `&[&str]` from the raw argc/argv data and calls the
/// user-defined `main`.
extern "C" fn _start_rust(argc: usize, argv_base: *const [u8; 16]) -> ! {
    // Each (ptr, len) pair is 16 bytes on x86_64, matching Rust's &str layout.
    // SAFETY: The kernel wrote valid (ptr, len) pairs and argc count.
    // The pointer is non-null and properly aligned (set up by the kernel on
    // a 16-byte aligned stack). The slice is valid for the lifetime of the
    // process.
    let args: &[&str] = unsafe { core::slice::from_raw_parts(argv_base.cast::<&str>(), argc) };

    unsafe extern "C" {
        fn main(args: &[&str]) -> i32;
    }
    // SAFETY: The user binary defines `main` as `#[unsafe(no_mangle)] pub extern "C" fn main`.
    let status = unsafe { main(args) };
    crate::sys::exit(status as usize);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    crate::eprintln!("PANIC: {}", info);
    crate::sys::exit(1);
}
