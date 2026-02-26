//! RFLAGS register.

bitflags::bitflags! {
    /// CPU flags (RFLAGS register).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RFlags: u64 {
        /// Carry flag.
        const CARRY           = 1 << 0;
        /// Parity flag.
        const PARITY          = 1 << 2;
        /// Adjust flag.
        const ADJUST          = 1 << 4;
        /// Zero flag.
        const ZERO            = 1 << 6;
        /// Sign flag.
        const SIGN            = 1 << 7;
        /// Trap flag (single-step).
        const TRAP            = 1 << 8;
        /// Interrupt enable flag.
        const INTERRUPT_FLAG  = 1 << 9;
        /// Direction flag.
        const DIRECTION       = 1 << 10;
        /// Overflow flag.
        const OVERFLOW        = 1 << 11;
        /// I/O privilege level (bit 0).
        const IOPL_0          = 1 << 12;
        /// I/O privilege level (bit 1).
        const IOPL_1          = 1 << 13;
        /// Resume flag.
        const RESUME          = 1 << 16;
        /// Alignment check / access control.
        const ALIGNMENT_CHECK = 1 << 18;
        /// ID flag (CPUID detection).
        const ID              = 1 << 21;
    }
}

/// Reads the current RFLAGS register value.
#[inline]
pub fn read() -> RFlags {
    let val: u64;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {}",
            out(reg) val,
            options(nomem),
        );
    }
    RFlags::from_bits_truncate(val)
}
