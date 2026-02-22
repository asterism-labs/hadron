//! QEMU exit device interface for benchmark binaries.
//!
//! On x86_64: isa-debug-exit at I/O port 0xf4.
//! Writing value N causes QEMU to exit with code `(N << 1) | 1`.
//! 0x10 -> exit 33 (success), 0x11 -> exit 35 (failure).

/// QEMU exit codes.
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum ExitCode {
    /// Success (QEMU exits with code 33).
    Success = 0x10,
    /// Failure (QEMU exits with code 35).
    Failure = 0x11,
}

/// Exit QEMU by writing to the exit device.
#[cfg(target_arch = "x86_64")]
pub fn exit(code: ExitCode) -> ! {
    // SAFETY: Writing to the isa-debug-exit I/O port terminates QEMU.
    // The infinite loop is unreachable but required for the `!` return type.
    unsafe {
        core::arch::asm!("out dx, eax",
            in("dx") 0xf4u16,
            in("eax") code as u32,
            options(nomem, nostack, preserves_flags));
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Exit QEMU (aarch64 stub).
#[cfg(target_arch = "aarch64")]
pub fn exit(_code: ExitCode) -> ! {
    todo!("aarch64 QEMU exit")
}
