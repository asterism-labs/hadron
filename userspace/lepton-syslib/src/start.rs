//! Entry point and panic handler for userspace binaries.
//!
//! Provides `_start` which reads argc/envc/argv/envp from the stack (placed
//! by the kernel), constructs argument and environment slices, initializes
//! the environment module, and calls the user-defined `main(args: &[&str]) -> i32`.

/// Naked entry point: reads argc, envc, and data base from the stack,
/// then calls `_start_rust`.
///
/// The kernel writes the following layout at RSP:
/// ```text
///   RSP + 0   → argc: usize
///   RSP + 8   → envc: usize
///   RSP + 16  → argv (ptr, len) pairs, then envp (ptr, len) pairs
/// ```
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "mov rdi, [rsp]",        // argc
        "mov rsi, [rsp + 8]",    // envc
        "lea rdx, [rsp + 16]",   // data_base (argv then envp)
        "call {start_rust}",
        start_rust = sym _start_rust,
    );
}

/// Rust entry point called from the naked `_start`.
///
/// Constructs `&[&str]` slices for argv and envp from the raw stack data,
/// initializes the environment, and calls the user-defined `main`.
extern "C" fn _start_rust(argc: usize, envc: usize, data_base: *const [u8; 16]) -> ! {
    // argv starts at data_base, envp starts at data_base + argc.
    // SAFETY: The kernel wrote valid (ptr, len) pairs and argc/envc counts.
    // The pointer is non-null and properly aligned (set up by the kernel on
    // a 16-byte aligned stack). The slices are valid for the lifetime of the
    // process.
    let args: &[&str] =
        unsafe { core::slice::from_raw_parts(data_base.cast::<&str>(), argc) };
    let envp: &[&str] =
        unsafe { core::slice::from_raw_parts(data_base.cast::<&str>().add(argc), envc) };

    // Initialize the environment variable map from envp.
    crate::env::init(envp);

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
