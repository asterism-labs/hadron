//! Kernel entry point called by the UEFI boot stub.

use hadron_boot_info::BootInfo;

/// Kernel entry point.
///
/// The UEFI boot stub jumps here after setting up page tables, loading
/// the kernel ELF, and building the `BootInfo` struct.
///
/// # Safety
///
/// `boot_info` must point to a valid, fully initialized `BootInfo` struct
/// in memory accessible under the kernel's page tables.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_init(_boot_info: *const BootInfo) -> ! {
    // Write "OK\n" to COM1 (0x3F8) to prove handoff works.
    unsafe {
        // SAFETY: Port 0x3F8 is the standard COM1 data register. Writing
        // bytes to it is safe during early boot (no contention).
        core::arch::asm!(
            "mov dx, 0x3F8",
            "mov al, 0x4F", // 'O'
            "out dx, al",
            "mov al, 0x4B", // 'K'
            "out dx, al",
            "mov al, 0x0A", // '\n'
            "out dx, al",
            options(nomem, nostack)
        );
    }
    loop {
        core::hint::spin_loop();
    }
}
