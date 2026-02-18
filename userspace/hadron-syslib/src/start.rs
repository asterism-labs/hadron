//! Entry point and panic handler for userspace binaries.
//!
//! Provides `_start` which calls the user-defined `main() -> i32` and then
//! exits with the returned status code.

/// C-ABI entry point called by the kernel's ELF loader.
///
/// Calls the user-defined `main` function and exits with its return value.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe extern "C" {
        fn main() -> i32;
    }
    // SAFETY: The user binary defines `main` as `#[unsafe(no_mangle)] pub extern "C" fn main`.
    let status = unsafe { main() };
    crate::sys::exit(status as usize);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    crate::eprintln!("PANIC: {}", info);
    crate::sys::exit(1);
}
