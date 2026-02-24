//! Interrupt-safe spin lock.
//!
//! Disables interrupts before acquiring the inner spinlock and restores
//! the previous interrupt state on release. This prevents deadlocks when
//! a lock is shared between interrupt handlers and normal kernel code.

use core::ops::{Deref, DerefMut};

#[cfg(hadron_lock_debug)]
use super::atomic::AtomicU32;
use super::atomic::{AtomicBool, Ordering};
use super::cell::UnsafeCell;

#[cfg(hadron_lockdep)]
use super::lockdep::LockClassId;

// ---------------------------------------------------------------------------
// IrqSpinLock nesting depth (per-CPU)
// ---------------------------------------------------------------------------

/// Per-CPU counter of currently held `IrqSpinLock`s. Used by `SpinLock` and
/// `Mutex` to assert they are not acquired inside an `IrqSpinLock` critical
/// section, which could cause deadlocks with interrupt handlers.
#[cfg(all(hadron_lock_debug, target_os = "none"))]
static IRQ_LOCK_DEPTH: crate::cpu_local::CpuLocal<AtomicU32> =
    crate::cpu_local::CpuLocal::new([const { AtomicU32::new(0) }; crate::cpu_local::MAX_CPUS]);

/// Returns the number of `IrqSpinLock`s held by the current CPU.
#[cfg(hadron_lock_debug)]
pub(super) fn irq_lock_depth() -> u32 {
    #[cfg(target_os = "none")]
    {
        if !crate::cpu_local::cpu_is_initialized() {
            return 0;
        }
        IRQ_LOCK_DEPTH.get().load(Ordering::Relaxed)
    }
    #[cfg(not(target_os = "none"))]
    {
        0
    }
}

/// Nesting depth threshold above which a warning is emitted. Deep nesting
/// increases contention windows and deadlock risk.
#[cfg(all(hadron_lock_debug, target_os = "none"))]
const IRQ_NESTING_WARN_THRESHOLD: u32 = 3;

#[cfg(all(hadron_lock_debug, target_os = "none"))]
fn increment_irq_depth() {
    if !crate::cpu_local::cpu_is_initialized() {
        return;
    }
    let prev = IRQ_LOCK_DEPTH.get().fetch_add(1, Ordering::Relaxed);
    #[cfg(hadron_lockdep)]
    if prev + 1 > IRQ_NESTING_WARN_THRESHOLD {
        super::lockdep::report_write(format_args!(
            "lockdep: IrqSpinLock nesting depth {} exceeds threshold {}",
            prev + 1,
            IRQ_NESTING_WARN_THRESHOLD,
        ));
    }
}

#[cfg(all(hadron_lock_debug, target_os = "none"))]
fn decrement_irq_depth() {
    if !crate::cpu_local::cpu_is_initialized() {
        return;
    }
    IRQ_LOCK_DEPTH.get().fetch_sub(1, Ordering::Relaxed);
}

/// A spin lock that disables interrupts while held.
pub struct IrqSpinLock<T> {
    locked: AtomicBool,
    #[cfg(hadron_lockdep)]
    name: &'static str,
    #[cfg(hadron_lockdep)]
    level: u8,
    data: UnsafeCell<T>,
}

// SAFETY: Same reasoning as SpinLock — atomic ops ensure exclusive access.
unsafe impl<T: Send> Send for IrqSpinLock<T> {}
unsafe impl<T: Send> Sync for IrqSpinLock<T> {}

impl<T> IrqSpinLock<T> {
    maybe_const_fn! {
        /// Creates a new unlocked `IrqSpinLock`.
        pub fn new(value: T) -> Self {
            Self {
                locked: AtomicBool::new(false),
                #[cfg(hadron_lockdep)]
                name: "<unnamed>",
                #[cfg(hadron_lockdep)]
                level: 0,
                data: UnsafeCell::new(value),
            }
        }
    }

    maybe_const_fn! {
        /// Creates a new unlocked `IrqSpinLock` with a name for lockdep diagnostics.
        pub fn named(name: &'static str, value: T) -> Self {
            Self {
                locked: AtomicBool::new(false),
                #[cfg(hadron_lockdep)]
                name,
                #[cfg(hadron_lockdep)]
                level: 0,
                data: UnsafeCell::new(value),
            }
        }
    }

    maybe_const_fn! {
        /// Creates a new unlocked `IrqSpinLock` with a name and lock ordering level.
        ///
        /// `level` is used for lockdep ordering checks: a lock at level N may
        /// only be acquired while holding locks at levels <= N.
        /// Level 0 means "unassigned" (no ordering check).
        pub fn leveled(name: &'static str, level: u8, value: T) -> Self {
            Self {
                locked: AtomicBool::new(false),
                #[cfg(hadron_lockdep)]
                name,
                #[cfg(hadron_lockdep)]
                level,
                data: UnsafeCell::new(value),
            }
        }
    }

    /// Acquires the lock, disabling interrupts first.
    pub fn lock(&self) -> IrqSpinLockGuard<'_, T> {
        // Save current RFLAGS and disable interrupts.
        let saved_flags = save_flags_and_cli();

        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        // TTAS spin to acquire.
        loop {
            if self
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                #[cfg(all(hadron_lock_debug, target_os = "none"))]
                increment_irq_depth();

                #[cfg(hadron_lockdep)]
                let class = self.lockdep_acquire();

                return IrqSpinLockGuard {
                    lock: self,
                    saved_flags,
                    #[cfg(hadron_lockdep)]
                    class,
                };
            }
            while self.locked.load(Ordering::Relaxed) {
                super::spin_wait_hint();
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
            #[cfg(all(hadron_lock_debug, target_os = "none"))]
            increment_irq_depth();

            #[cfg(hadron_lockdep)]
            let class = self.lockdep_acquire();

            Some(IrqSpinLockGuard {
                lock: self,
                saved_flags,
                #[cfg(hadron_lockdep)]
                class,
            })
        } else {
            // Failed — restore flags.
            restore_flags(saved_flags);
            None
        }
    }

    /// Registers this lock with lockdep and records the acquisition.
    #[cfg(hadron_lockdep)]
    fn lockdep_acquire(&self) -> LockClassId {
        let class = super::lockdep::get_or_register_leveled(
            self as *const _ as usize,
            self.level,
            self.name,
            super::lockdep::LockKind::IrqSpinLock,
        );
        super::lockdep::lock_acquired(class);
        class
    }
}

/// RAII guard that restores interrupt state on drop.
pub struct IrqSpinLockGuard<'a, T> {
    lock: &'a IrqSpinLock<T>,
    saved_flags: u64,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

impl<T> Deref for IrqSpinLockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: The lock is held, so we have exclusive access to the data.
        self.lock.data.with(|ptr| unsafe { &*ptr })
    }
}

impl<T> DerefMut for IrqSpinLockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The lock is held, so we have exclusive access to the data.
        self.lock.data.with_mut(|ptr| unsafe { &mut *ptr })
    }
}

impl<T> Drop for IrqSpinLockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);

        #[cfg(hadron_lock_stress)]
        super::stress::stress_delay();

        #[cfg(hadron_lockdep)]
        if self.class != LockClassId::NONE {
            super::lockdep::lock_released(self.class);
        }

        #[cfg(all(hadron_lock_debug, target_os = "none"))]
        decrement_irq_depth();

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

#[cfg(all(not(target_os = "none"), not(loom)))]
#[inline]
fn save_flags_and_cli() -> u64 {
    0
}

#[cfg(all(not(target_os = "none"), not(loom)))]
#[inline]
fn restore_flags(_flags: u64) {}

#[cfg(loom)]
#[inline]
fn save_flags_and_cli() -> u64 {
    super::loom_mock::mock_save_flags_and_cli()
}

#[cfg(loom)]
#[inline]
fn restore_flags(flags: u64) {
    super::loom_mock::mock_restore_flags(flags);
}

#[cfg(loom)]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;

    use super::super::loom_mock;
    use super::IrqSpinLock;

    #[test]
    fn loom_irq_spinlock_mutual_exclusion() {
        loom::model(|| {
            let lock = Arc::new(IrqSpinLock::new(0usize));

            let threads: Vec<_> = (0..2)
                .map(|_| {
                    let lock = lock.clone();
                    thread::spawn(move || {
                        for _ in 0..2 {
                            let mut guard = lock.lock();
                            *guard += 1;
                        }
                    })
                })
                .collect();

            for t in threads {
                t.join().unwrap();
            }

            assert_eq!(*lock.lock(), 4);
        });
    }

    #[test]
    fn loom_irq_spinlock_interrupt_state() {
        loom::model(|| {
            let lock = IrqSpinLock::new(42usize);

            // Interrupts start enabled.
            assert!(loom_mock::mock_irq_enabled());

            {
                let guard = lock.lock();
                // Interrupts should be disabled while lock is held.
                assert!(!loom_mock::mock_irq_enabled());
                assert_eq!(*guard, 42);
            }

            // Interrupts should be restored after guard is dropped.
            assert!(loom_mock::mock_irq_enabled());
        });
    }
}
