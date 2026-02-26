//! Time Stamp Counter (TSC) reading primitives.
//!
//! Provides `rdtsc` and `rdtscp` wrappers for high-resolution timing.

/// Reads the TSC (Time Stamp Counter) using `RDTSC`.
///
/// Returns the 64-bit timestamp. Note: this is not serializing --
/// the CPU may reorder it relative to surrounding instructions.
#[inline]
pub fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: RDTSC is available on all x86_64 processors and has no side effects.
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(hi) << 32) | u64::from(lo)
}

/// Reads the TSC using `RDTSCP`, which is serializing.
///
/// Returns `(timestamp, processor_id)` where `processor_id` is the
/// value of `IA32_TSC_AUX` (typically the logical processor number).
#[inline]
pub fn read_tscp() -> (u64, u32) {
    let lo: u32;
    let hi: u32;
    let aux: u32;
    // SAFETY: RDTSCP is available on modern x86_64 processors and has no side effects.
    unsafe {
        core::arch::asm!(
            "rdtscp",
            out("eax") lo,
            out("edx") hi,
            out("ecx") aux,
            options(nomem, nostack, preserves_flags),
        );
    }
    ((u64::from(hi) << 32) | u64::from(lo), aux)
}
