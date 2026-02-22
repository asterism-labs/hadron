//! Lock contention stress delays.
//!
//! Injects random spin delays around lock acquire/release to widen race
//! windows and surface timing-dependent bugs. Gated behind
//! `cfg(hadron_lock_stress)`.
//!
//! ## Design
//!
//! - **PRNG**: xorshift64, per-CPU state via `CpuLocal<AtomicU64>` — no locking.
//! - **Timing**: a registered `fn() -> u64` nanosecond callback (set by the
//!   kernel after HPET init). Before the callback is registered, delays are
//!   no-ops so early boot is unaffected.
//! - **Delay**: spins for a random duration in `[0, max_us)` microseconds.

use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering};

use crate::cpu_local::{CpuLocal, MAX_CPUS};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum stress delay in microseconds. Set by `init()`.
static MAX_US: AtomicU32 = AtomicU32::new(10);

/// Nanosecond clock callback. Null until the kernel registers one.
static NANOS_FN: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());

/// Per-CPU xorshift64 PRNG state.
static PRNG_STATE: CpuLocal<AtomicU64> = CpuLocal::new([const { AtomicU64::new(0) }; MAX_CPUS]);

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initializes the stress delay subsystem.
///
/// - `max_us`: maximum random delay in microseconds (from `LOCK_STRESS_MAX_US`).
/// - `seed`: initial PRNG seed (e.g., `boot_nanos()`). If 0, a fallback
///   constant is used to avoid a stuck-at-zero xorshift.
pub fn init(max_us: u32, seed: u64) {
    MAX_US.store(max_us, Ordering::Relaxed);

    let base = if seed == 0 {
        0xDEAD_BEEF_CAFE_BABEu64
    } else {
        seed
    };

    // Seed each CPU with a divergent value.
    for i in 0..MAX_CPUS {
        let cpu_seed = base
            .wrapping_add(i as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
        // Avoid zero (xorshift fixed point).
        let cpu_seed = if cpu_seed == 0 {
            base ^ 0x1234_5678
        } else {
            cpu_seed
        };
        PRNG_STATE
            .get_for(i as u32)
            .store(cpu_seed, Ordering::Relaxed);
    }
}

/// Registers the nanosecond clock function.
///
/// Before this is called, [`stress_delay`] is a no-op. The kernel should
/// call this after HPET initialization:
///
/// ```ignore
/// hadron_core::sync::stress::set_nanos_fn(crate::time::boot_nanos);
/// ```
///
/// # Safety
///
/// The provided function must be safe to call from any context (including
/// interrupt handlers) and must not acquire any lock tracked by lockdep.
pub unsafe fn set_nanos_fn(f: fn() -> u64) {
    NANOS_FN.store(f as *mut (), Ordering::Release);
}

// ---------------------------------------------------------------------------
// PRNG
// ---------------------------------------------------------------------------

/// Returns the next pseudo-random u64 for the current CPU.
#[inline]
fn next_random() -> u64 {
    let state = PRNG_STATE.get();
    let mut x = state.load(Ordering::Relaxed);
    if x == 0 {
        x = 0xDEAD_BEEF_CAFE_BABEu64;
    }
    // xorshift64
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    state.store(x, Ordering::Relaxed);
    x
}

// ---------------------------------------------------------------------------
// Delay
// ---------------------------------------------------------------------------

/// Spins for a random duration in `[0, max_us)` microseconds.
///
/// Uses the registered nanosecond clock to measure elapsed time. If no
/// clock is registered yet (early boot), this is a no-op.
///
/// # Safety considerations
///
/// This function must NOT acquire any lock (would cause infinite recursion
/// when called from lock acquire/release paths).
#[inline]
pub fn stress_delay() {
    let nanos_ptr = NANOS_FN.load(Ordering::Acquire);
    if nanos_ptr.is_null() {
        return; // No clock yet — no-op during early boot.
    }

    let max_us = MAX_US.load(Ordering::Relaxed);
    if max_us == 0 {
        return;
    }

    let target_ns = next_random() % (max_us as u64 * 1000);
    if target_ns == 0 {
        return;
    }

    // SAFETY: The pointer was set by `set_nanos_fn` from a valid `fn() -> u64`.
    let nanos_fn: fn() -> u64 = unsafe { core::mem::transmute(nanos_ptr) };

    let start = nanos_fn();
    if start == 0 {
        // Clock not yet ticking — fall back to a simple spin count.
        let spin_count = target_ns / 10; // rough approximation
        for _ in 0..spin_count {
            core::hint::spin_loop();
        }
        return;
    }

    while nanos_fn().wrapping_sub(start) < target_ns {
        core::hint::spin_loop();
    }
}
