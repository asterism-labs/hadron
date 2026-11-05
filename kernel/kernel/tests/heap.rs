//! Boots the kernel and allocates from the heap on the main thread.
//!
//! An integration test is a whole kernel image with its own entry point
//! and panic handler. `gluon test` boots it on its own, and it passes
//! when QEMU exits with success.

#![cfg_attr(target_os = "none", no_std, no_main)]

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod entry {
    extern crate alloc;

    use alloc::boxed::Box;
    use alloc::vec::Vec;
    use core::panic::PanicInfo;

    /// Entry point. Limine jumps here, as named by `ENTRY` in `kernel.ld`.
    #[unsafe(no_mangle)]
    extern "C" fn kmain() -> ! {
        // SAFETY: Limine jumps here, and nothing runs before it.
        unsafe { kernel::boot::limine::enter(main) }
    }

    fn main() {
        let boxed = Box::new(42_u64);
        assert_eq!(*boxed, 42);
        let squares: Vec<u64> = (0..1024).map(|i| i * i).collect();
        assert_eq!(squares.iter().sum::<u64>(), 357_389_824);
    }

    #[panic_handler]
    fn panic(info: &PanicInfo<'_>) -> ! {
        kernel::panic::handle(info)
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
