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
fn panic(info: &core::panic::PanicInfo) -> ! {
    // Write panic info to COM1 for debugging.
    serial_str("KERNEL PANIC: ");
    if let Some(loc) = info.location() {
        serial_str(loc.file());
        serial_str(":");
        // Simple decimal formatting for line number.
        let mut buf = [0u8; 10];
        let s = fmt_u32(loc.line(), &mut buf);
        serial_str(s);
    }
    serial_str("\n");
    loop {
        core::hint::spin_loop();
    }
}

fn serial_str(s: &str) {
    for b in s.bytes() {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") 0x3F8u16,
                in("al") b,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

fn fmt_u32(mut n: u32, buf: &mut [u8; 10]) -> &str {
    if n == 0 {
        return "0";
    }
    let mut i = 10;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    // SAFETY: digits are all ASCII.
    unsafe { core::str::from_utf8_unchecked(&buf[i..]) }
}
