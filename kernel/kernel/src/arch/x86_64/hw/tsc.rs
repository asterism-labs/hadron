//! Time Stamp Counter (TSC) reading primitives.
//!
//! Re-exports from [`hadron_arch_x86_64::instructions::misc`].

use hadron_arch_x86_64::instructions::misc;

/// Reads the TSC (Time Stamp Counter) using `RDTSC`.
///
/// Returns the 64-bit timestamp. Note: this is not serializing --
/// the CPU may reorder it relative to surrounding instructions.
#[inline]
pub fn read_tsc() -> u64 {
    misc::rdtsc()
}

/// Reads the TSC using `RDTSCP`, which is serializing.
///
/// Returns `(timestamp, processor_id)` where `processor_id` is the
/// value of `IA32_TSC_AUX` (typically the logical processor number).
#[inline]
pub fn read_tscp() -> (u64, u32) {
    misc::rdtscp()
}
