//! CRT entry point for programs linked against `hadron-libc`.
//!
//! Reads the C-style argc/argv/envp from the initial stack layout
//! (placed by the kernel), initializes libc subsystems, and calls
//! `main(argc, argv, envp)` with the standard C signature.

/// Naked entry point: passes the raw stack pointer to `_start_rust`.
///
/// The kernel writes the following C ABI layout at RSP:
/// ```text
///   RSP + 0             → argc: usize
///   RSP + 8             → argv[0]: *const c_char
///   ...
///   RSP + 8*(argc+1)    → NULL (argv terminator)
///   RSP + 8*(argc+2)    → envp[0]: *const c_char
///   ...
///   (terminated by NULL)
/// ```
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "mov rdi, rsp", // pass original stack pointer (points to argc)
        "and rsp, -16", // ensure 16-byte alignment for call
        "call {start_rust}",
        start_rust = sym _start_rust,
    );
}

/// Rust entry point called from the naked `_start`.
///
/// Reads C-style argc/argv/envp, initializes libc, calls `main()`.
extern "C" fn _start_rust(stack: *const usize) -> ! {
    // SAFETY: The kernel wrote a valid C-style argv/envp layout on the stack.
    let argc = unsafe { *stack } as i32;
    let argv = unsafe { stack.add(1) } as *const *const u8;
    // envp starts after argv[argc] + NULL sentinel
    let envp = unsafe { argv.add(argc as usize + 1) } as *const *const u8;

    // Initialize libc subsystems.
    // 1. Set up environ from envp.
    unsafe { crate::env::init_environ(envp) };

    // 2. Initialize stdio (streams are const-initialized, just mark ready).
    crate::stdio::init();

    // 3. Call main.
    unsafe extern "C" {
        fn main(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32;
    }
    // SAFETY: The user binary defines `main` with the standard C signature.
    let status = unsafe { main(argc, argv, envp) };

    // 4. Call exit (runs atexit handlers, flushes stdio, terminates).
    unsafe { crate::process::exit(status) }
}
