//! Entry point and panic handler for userspace binaries.
//!
//! Provides `_start` which reads the standard C-style argc/argv/envp layout
//! from the stack (placed by the kernel), converts to Rust `&[&str]` slices,
//! initializes the environment module, and calls `main(args: &[&str]) -> i32`.

/// Naked entry point: passes the raw stack pointer to `_start_rust`.
///
/// The kernel writes the following C ABI layout at RSP:
/// ```text
///   RSP + 0   → argc: usize
///   RSP + 8   → argv[0]: *const c_char
///   ...
///   RSP + 8*(argc+1)  → NULL (argv terminator)
///   RSP + 8*(argc+2)  → envp[0]: *const c_char
///   ...
///   (terminated by NULL)
/// ```
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub extern "C" fn _start() -> ! {
    core::arch::naked_asm!(
        "mov rdi, rsp",          // pass original stack pointer (points to argc)
        "and rsp, -16",          // ensure 16-byte alignment for call
        "call {start_rust}",
        start_rust = sym _start_rust,
    );
}

/// Rust entry point called from the naked `_start`.
///
/// Reads the C-style argc/argv/envp layout, converts null-terminated C strings
/// to `&str` slices, initializes the environment, and calls `main`.
extern "C" fn _start_rust(stack: *const usize) -> ! {
    // SAFETY: The kernel wrote a valid C-style argv/envp layout on the stack.
    // Pointers are non-null and point to valid null-terminated UTF-8 strings.
    let argc = unsafe { *stack };
    let argv = unsafe { stack.add(1) } as *const *const u8;
    // envp starts after argv[argc] + NULL sentinel
    let envp = unsafe { argv.add(argc + 1) } as *const *const u8;

    // Count envp entries (walk until NULL sentinel).
    let mut envc = 0;
    // SAFETY: envp is a NULL-terminated pointer array written by the kernel.
    unsafe {
        while !(*envp.add(envc)).is_null() {
            envc += 1;
        }
    }

    // Convert C strings to &str slices using fixed-size stack buffers.
    const MAX_ARGS: usize = 32;
    const MAX_ENVS: usize = 64;
    let mut arg_strs: [&str; MAX_ARGS] = [""; MAX_ARGS];
    let mut env_strs: [&str; MAX_ENVS] = [""; MAX_ENVS];

    let argc = argc.min(MAX_ARGS);
    let envc = envc.min(MAX_ENVS);

    for i in 0..argc {
        // SAFETY: argv[i] is a valid, non-null pointer to a NUL-terminated
        // UTF-8 string written by the kernel.
        unsafe {
            let ptr = *argv.add(i);
            let len = c_strlen(ptr);
            arg_strs[i] = core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len));
        }
    }

    for i in 0..envc {
        // SAFETY: envp[i] is a valid, non-null pointer to a NUL-terminated
        // UTF-8 string written by the kernel.
        unsafe {
            let ptr = *envp.add(i);
            let len = c_strlen(ptr);
            env_strs[i] = core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len));
        }
    }

    let args = &arg_strs[..argc];
    let envs = &env_strs[..envc];

    // Initialize the environment variable map from envp.
    crate::env::init(envs);

    unsafe extern "C" {
        fn main(args: &[&str]) -> i32;
    }
    // SAFETY: The user binary defines `main` as `#[unsafe(no_mangle)] pub extern "C" fn main`.
    let status = unsafe { main(args) };
    crate::sys::exit(status as usize);
}

/// Compute the length of a null-terminated C string.
///
/// # Safety
///
/// `s` must point to a valid, null-terminated byte sequence.
unsafe fn c_strlen(s: *const u8) -> usize {
    let mut len = 0;
    // SAFETY: Caller guarantees s points to a null-terminated string.
    while unsafe { *s.add(len) } != 0 {
        len += 1;
    }
    len
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    crate::eprintln!("PANIC: {}", info);
    crate::sys::exit(1);
}
