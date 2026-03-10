//! Minimal UEFI boot stub for Hadron.
//!
//! Prints a greeting via SimpleTextOutput and halts.

#![no_std]
#![no_main]

use core::fmt::Write;

use uefi::EfiHandle;
use uefi::EfiStatus;
use uefi::api::{Boot, SystemTable};
use uefi::table;

/// UEFI application entry point.
#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(handle: EfiHandle, system_table: *mut table::SystemTable) -> EfiStatus {
    // SAFETY: `handle` and `system_table` are provided by UEFI firmware at boot
    // and are valid for the duration of the boot phase.
    let st = unsafe { SystemTable::<Boot>::from_raw(handle, system_table) };

    let mut console = st.console_out();
    let _ = console.clear_screen();
    let _ = write!(console, "Hello from Hadron!\n");

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
