//! Model Specific Registers (MSRs).

/// A Model Specific Register, identified by its address.
#[derive(Debug, Clone, Copy)]
pub struct Msr(u32);

/// IA32_EFER MSR address.
pub const IA32_EFER: Msr = Msr(0xC000_0080);

/// IA32_PAT MSR address.
pub const IA32_PAT: Msr = Msr(0x0000_0277);

/// STAR MSR — SYSCALL/SYSRET segment selectors.
pub const MSR_STAR: Msr = Msr(0xC000_0081);

/// LSTAR MSR — SYSCALL entry point (RIP).
pub const MSR_LSTAR: Msr = Msr(0xC000_0082);

/// SFMASK MSR — RFLAGS mask applied on SYSCALL entry.
pub const MSR_SFMASK: Msr = Msr(0xC000_0084);

/// IA32_GS_BASE MSR — current GS base address.
pub const IA32_GS_BASE: Msr = Msr(0xC000_0101);

/// IA32_KERNEL_GS_BASE MSR — swapped with GS_BASE by `swapgs`.
pub const IA32_KERNEL_GS_BASE: Msr = Msr(0xC000_0102);

bitflags::bitflags! {
    /// IA32_EFER register flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct EferFlags: u64 {
        /// System Call Extensions (SYSCALL/SYSRET).
        const SYSTEM_CALL_ENABLE = 1 << 0;
        /// Long Mode Enable.
        const LONG_MODE_ENABLE   = 1 << 8;
        /// No-Execute Enable.
        const NO_EXECUTE_ENABLE  = 1 << 11;
    }
}

impl Msr {
    /// Creates a new MSR from its address.
    #[inline]
    pub const fn new(addr: u32) -> Self {
        Self(addr)
    }

    /// Reads the 64-bit value of this MSR.
    ///
    /// # Safety
    ///
    /// The caller must ensure this MSR address is valid and readable.
    #[inline]
    pub unsafe fn read(self) -> u64 {
        let (low, high): (u32, u32);
        unsafe {
            core::arch::asm!(
                "rdmsr",
                in("ecx") self.0,
                out("eax") low,
                out("edx") high,
                options(nomem, nostack, preserves_flags),
            );
        }
        u64::from(high) << 32 | u64::from(low)
    }

    /// Writes a 64-bit value to this MSR.
    ///
    /// # Safety
    ///
    /// The caller must ensure this MSR address is valid and the value is
    /// appropriate.
    #[inline]
    pub unsafe fn write(self, value: u64) {
        let low = value as u32;
        let high = (value >> 32) as u32;
        unsafe {
            core::arch::asm!(
                "wrmsr",
                in("ecx") self.0,
                in("eax") low,
                in("edx") high,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}
