//! Interrupt-safe spin lock.
//!
//! Disables interrupts before acquiring the inner spinlock and restores
//! the previous interrupt state on release. This prevents deadlocks when
//! a lock is shared between interrupt handlers and normal kernel code.

use core::ops::{Deref, DerefMut};
use core::sync::atomic::Ordering;

#[cfg(hadron_lock_debug)]
use super::atomic::AtomicU32;
use super::backend::{AtomicBoolOps, Backend, CoreBackend, IrqBackend, UnsafeCellOps};

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

// ─── Type aliases ─────────────────────────────────────────────────────

/// A spin lock that disables interrupts while held.
pub type IrqSpinLock<T> = IrqSpinLockInner<T, CoreBackend>;

/// RAII guard that restores interrupt state on drop.
pub type IrqSpinLockGuard<'a, T> = IrqSpinLockGuardInner<'a, T, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic interrupt-safe spin lock.
///
/// Use [`IrqSpinLock`] (the [`CoreBackend`] alias) in production code.
pub struct IrqSpinLockInner<T, B: Backend> {
    locked: B::AtomicBool,
    #[cfg(hadron_lockdep)]
    name: &'static str,
    #[cfg(hadron_lockdep)]
    level: u8,
    data: B::UnsafeCell<T>,
}

// SAFETY: Same reasoning as SpinLock — atomic ops ensure exclusive access.
unsafe impl<T: Send, B: Backend> Send for IrqSpinLockInner<T, B> {}
unsafe impl<T: Send, B: Backend> Sync for IrqSpinLockInner<T, B> {}

// ─── Const constructors (CoreBackend only) ────────────────────────────

impl<T> IrqSpinLock<T> {
    /// Creates a new unlocked `IrqSpinLock`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            #[cfg(hadron_lockdep)]
            level: 0,
            data: core::cell::UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `IrqSpinLock` with a name for lockdep diagnostics.
    pub const fn named(name: &'static str, value: T) -> Self {
        let _ = name;
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name,
            #[cfg(hadron_lockdep)]
            level: 0,
            data: core::cell::UnsafeCell::new(value),
        }
    }

    /// Creates a new unlocked `IrqSpinLock` with a name and lock ordering level.
    ///
    /// `level` is used for lockdep ordering checks: a lock at level N may
    /// only be acquired while holding locks at levels <= N.
    /// Level 0 means "unassigned" (no ordering check).
    pub const fn leveled(name: &'static str, level: u8, value: T) -> Self {
        let _ = (name, level);
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            #[cfg(hadron_lockdep)]
            name,
            #[cfg(hadron_lockdep)]
            level,
            data: core::cell::UnsafeCell::new(value),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<T, B: Backend> IrqSpinLockInner<T, B> {
    /// Creates a new unlocked `IrqSpinLockInner` using backend factory functions.
    pub fn new_with_backend(value: T) -> Self {
        Self {
            locked: B::new_atomic_bool(false),
            #[cfg(hadron_lockdep)]
            name: "<unnamed>",
            #[cfg(hadron_lockdep)]
            level: 0,
            data: B::new_unsafe_cell(value),
        }
    }
}

// ─── Algorithm (generic over B: IrqBackend) ───────────────────────────

impl<T, B: IrqBackend> IrqSpinLockInner<T, B> {
    /// Acquires the lock, disabling interrupts first.
    pub fn lock(&self) -> IrqSpinLockGuardInner<'_, T, B> {
        // Save current RFLAGS and disable interrupts.
        let saved_flags = B::save_flags_and_cli();

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

                return IrqSpinLockGuardInner {
                    lock: self,
                    saved_flags,
                    #[cfg(hadron_lockdep)]
                    class,
                };
            }
            while self.locked.load(Ordering::Relaxed) {
                B::spin_wait_hint();
            }
        }
    }

    /// Attempts to acquire the lock without blocking.
    pub fn try_lock(&self) -> Option<IrqSpinLockGuardInner<'_, T, B>> {
        let saved_flags = B::save_flags_and_cli();
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            #[cfg(all(hadron_lock_debug, target_os = "none"))]
            increment_irq_depth();

            #[cfg(hadron_lockdep)]
            let class = self.lockdep_acquire();

            Some(IrqSpinLockGuardInner {
                lock: self,
                saved_flags,
                #[cfg(hadron_lockdep)]
                class,
            })
        } else {
            // Failed — restore flags.
            B::restore_flags(saved_flags);
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

// ─── Guard ────────────────────────────────────────────────────────────

/// RAII guard that restores interrupt state on drop.
pub struct IrqSpinLockGuardInner<'a, T, B: IrqBackend> {
    lock: &'a IrqSpinLockInner<T, B>,
    saved_flags: u64,
    #[cfg(hadron_lockdep)]
    class: LockClassId,
}

impl<T, B: IrqBackend> Deref for IrqSpinLockGuardInner<'_, T, B> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: The lock is held, so we have exclusive access to the data.
        self.lock.data.with(|ptr| unsafe { &*ptr })
    }
}

impl<T, B: IrqBackend> DerefMut for IrqSpinLockGuardInner<'_, T, B> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: The lock is held, so we have exclusive access to the data.
        self.lock.data.with_mut(|ptr| unsafe { &mut *ptr })
    }
}

impl<T, B: IrqBackend> Drop for IrqSpinLockGuardInner<'_, T, B> {
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

        B::restore_flags(self.saved_flags);
    }
}

/// !Send — must not be sent across threads (interrupt state is per-CPU).
impl<T, B: IrqBackend> !Send for IrqSpinLockGuardInner<'_, T, B> {}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(loom)]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;

    use super::IrqSpinLockInner;
    use crate::sync::backend::LoomBackend;
    use crate::sync::loom_mock;

    type LoomIrqSpinLock<T> = IrqSpinLockInner<T, LoomBackend>;

    #[test]
    fn loom_irq_spinlock_mutual_exclusion() {
        loom::model(|| {
            let lock = Arc::new(LoomIrqSpinLock::new_with_backend(0usize));

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
            let lock = LoomIrqSpinLock::new_with_backend(42usize);

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

#[cfg(shuttle)]
mod shuttle_tests {
    use shuttle::sync::Arc;
    use shuttle::thread;

    use super::IrqSpinLockInner;
    use crate::sync::backend::ShuttleBackend;

    type ShuttleIrqSpinLock<T> = IrqSpinLockInner<T, ShuttleBackend>;

    #[test]
    fn shuttle_three_thread_mutual_exclusion() {
        shuttle::check_random(
            || {
                let lock = Arc::new(ShuttleIrqSpinLock::new_with_backend(0usize));

                let threads: Vec<_> = (0..3)
                    .map(|_| {
                        let lock = lock.clone();
                        thread::spawn(move || {
                            for _ in 0..5 {
                                let mut guard = lock.lock();
                                *guard += 1;
                            }
                        })
                    })
                    .collect();

                for t in threads {
                    t.join().unwrap();
                }

                assert_eq!(*lock.lock(), 15);
            },
            100,
        );
    }

    #[test]
    fn shuttle_irq_state_preserved() {
        shuttle::check_random(
            || {
                use crate::sync::shuttle_mock;
                let lock = Arc::new(ShuttleIrqSpinLock::new_with_backend(42usize));

                assert!(shuttle_mock::mock_irq_enabled());

                {
                    let guard = lock.lock();
                    assert!(!shuttle_mock::mock_irq_enabled());
                    assert_eq!(*guard, 42);
                }

                assert!(shuttle_mock::mock_irq_enabled());
            },
            100,
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Verify that `try_lock` returns `None` when the lock is already held.
    #[kani::proof]
    fn irq_spinlock_try_lock_semantics() {
        let lock = IrqSpinLock::new(0u32);
        let guard = lock.try_lock();
        assert!(guard.is_some());
        // While held, a second try_lock must fail.
        assert!(lock.try_lock().is_none());
        drop(guard);
        // After release, try_lock must succeed again.
        assert!(lock.try_lock().is_some());
    }

    /// Verify that data written under the lock is visible after re-acquisition.
    #[kani::proof]
    fn irq_spinlock_protects_data() {
        let val: u32 = kani::any();
        let lock = IrqSpinLock::new(0u32);
        {
            let mut guard = lock.lock();
            *guard = val;
        }
        let guard = lock.lock();
        assert_eq!(*guard, val);
    }
}
