//! Thin binary wrapper that links `hadron-kernel` into a standalone ELF.
//!
//! The actual kernel entry point (`kernel_init`) lives in `hadron_kernel::entry`.
//! This crate exists solely to produce a linked ELF binary with the kernel
//! linker script applied. The resulting binary is embedded into the UEFI boot
//! stub at build time.

#![no_std]
#![no_main]

extern crate hadron_kernel;

// Force the linker to retain `kernel_init` (the entry point declared in the
// linker script) even though nothing in *this* crate references it directly.
core::arch::global_asm!(".global kernel_init");

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
