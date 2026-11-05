//! The Hadron kernel binary: the entry point Limine jumps to and the panic
//! handler. The kernel itself is the `kernel` library.

#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod entry {
    use core::panic::PanicInfo;

    /// Entry point. Limine jumps here, as named by `ENTRY` in `kernel.ld`.
    #[unsafe(no_mangle)]
    extern "C" fn kmain() -> ! {
        // SAFETY: Limine jumps here, and nothing runs before it.
        unsafe { kernel::boot::limine::enter() }
    }

    #[panic_handler]
    fn panic(info: &PanicInfo<'_>) -> ! {
        kernel::panic::handle(info)
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
