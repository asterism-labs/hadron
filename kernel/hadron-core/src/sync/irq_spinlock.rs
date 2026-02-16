//! Interrupt-safe spin lock.
//!
//! Disables interrupts before acquiring the inner spinlock and restores
//! the previous interrupt state on release. This prevents deadlocks when
//! a lock is shared between interrupt handlers and normal kernel code.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A spin lock that disables interrupts while held.
pub struct IrqSpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// SAFETY: Same reasoning as SpinLock — atomic ops ensure exclusive access.
unsafe impl<T: Send> Send for IrqSpinLock<T> {}
unsafe impl<T: Send> Sync for IrqSpinLock<T> {}

impl<T> IrqSpinLock<T> {
    /// Creates a new unlocked `IrqSpinLock`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    /// Acquires the lock, disabling interrupts first.
    pub fn lock(&self) -> IrqSpinLockGuard<'_, T> {
        // Save current RFLAGS and disable interrupts.
        let saved_flags = save_flags_and_cli();

        // TTAS spin to acquire.
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return IrqSpinLockGuard {
                    lock: self,
                    saved_flags,
                };
            }
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    /// Attempts to acquire the lock without blocking.
    pub fn try_lock(&self) -> Option<IrqSpinLockGuard<'_, T>> {
        let saved_flags = save_flags_and_cli();
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(IrqSpinLockGuard {
                lock: self,
                saved_flags,
            })
        } else {
            // Failed — restore flags.
            restore_flags(saved_flags);
            None
        }
    }
}

/// RAII guard that restores interrupt state on drop.
pub struct IrqSpinLockGuard<'a, T> {
    lock: &'a IrqSpinLock<T>,
    saved_flags: u64,
}

impl<T> Deref for IrqSpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: The lock is held, so we have exclusive access to the data.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for IrqSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The lock is held, so we have exclusive access to the data.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for IrqSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        restore_flags(self.saved_flags);
    }
}

/// !Send — must not be sent across threads (interrupt state is per-CPU).
impl<T> !Send for IrqSpinLockGuard<'_, T> {}

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
#[inline]
fn save_flags_and_cli() -> u64 {
    let flags: u64;
    // SAFETY: Reading RFLAGS and disabling interrupts is safe in kernel mode.
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {}",
            "cli",
            out(reg) flags,
            options(nomem),
        );
    }
    flags
}

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
#[inline]
fn restore_flags(flags: u64) {
    // Only restore the IF bit — push full flags and use popfq.
    if flags & (1 << 9) != 0 {
        // SAFETY: Re-enabling interrupts is safe; we are restoring a previous state.
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
        }
    }
}

#[cfg(all(target_os = "none", target_arch = "aarch64"))]
#[inline]
fn save_flags_and_cli() -> u64 {
    let flags: u64;
    // SAFETY: Reading DAIF and masking interrupts is safe in kernel mode.
    unsafe {
        core::arch::asm!(
            "mrs {}, DAIF",
            "msr DAIFSet, #0xf",
            out(reg) flags,
            options(nomem),
        );
    }
    flags
}

#[cfg(all(target_os = "none", target_arch = "aarch64"))]
#[inline]
fn restore_flags(flags: u64) {
    // SAFETY: Restoring DAIF is safe; we are restoring a previous state.
    unsafe {
        core::arch::asm!(
            "msr DAIF, {}",
            in(reg) flags,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[cfg(not(target_os = "none"))]
#[inline]
fn save_flags_and_cli() -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
#[inline]
fn restore_flags(_flags: u64) {}
