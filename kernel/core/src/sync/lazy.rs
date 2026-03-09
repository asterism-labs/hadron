//! Lazy initialization primitive for `no_std`.
//!
//! Provides [`LazyLock`], a `no_std` equivalent of `std::sync::LazyLock`
//! that initializes a value on first access using a spin-based state machine.

use core::mem::MaybeUninit;
use core::ops::Deref;
use core::sync::atomic::Ordering;

use super::backend::{AtomicIntOps, Backend, CoreBackend, UnsafeCellOps};

const UNINIT: u8 = 0;
const INITIALIZING: u8 = 1;
const READY: u8 = 2;
/// Poisoned state: the init closure panicked during initialization.
///
/// Defense-in-depth — with `panic = abort` (our target), a panic during
/// init terminates the kernel immediately and this state is never observed.
/// It exists so that if this code is ever used in a test harness with
/// `panic = unwind`, waiters don't spin forever.
const POISONED: u8 = 3;

// ─── Type alias ───────────────────────────────────────────────────────

/// A value that is initialized on first access.
///
/// Thread-safe via an atomic state machine. If multiple threads race to
/// initialize, one wins and the others spin until the value is ready.
///
/// # Panic safety
///
/// If the init closure panics, the state transitions to [`POISONED`] and
/// subsequent accesses panic immediately. Under the kernel's `panic = abort`
/// configuration this is moot — the kernel halts on the first panic — but
/// the poisoning logic provides defense-in-depth for non-abort contexts
/// (e.g. host-side unit tests).
pub type LazyLock<T, F = fn() -> T> = LazyLockInner<T, F, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic lazy lock.
pub struct LazyLockInner<T, F, B: Backend> {
    state: B::AtomicU8,
    value: B::UnsafeCell<MaybeUninit<T>>,
    init: B::UnsafeCell<Option<F>>,
}

// SAFETY: The atomic state machine ensures that the value is fully initialized
// before any thread can read it, and that the init closure is consumed exactly
// once.
unsafe impl<T: Send + Sync, F: Send, B: Backend> Send for LazyLockInner<T, F, B> {}
unsafe impl<T: Send + Sync, F: Send, B: Backend> Sync for LazyLockInner<T, F, B> {}

/// Guard that poisons the `LazyLock` if dropped without completing init.
struct InitGuard<'a, B: Backend> {
    state: &'a B::AtomicU8,
}

impl<'a, B: Backend> InitGuard<'a, B> {
    fn new(state: &'a B::AtomicU8) -> Self {
        Self { state }
    }

    /// Disarm the guard after successful initialization.
    fn defuse(self) {
        core::mem::forget(self);
    }
}

impl<B: Backend> Drop for InitGuard<'_, B> {
    fn drop(&mut self) {
        self.state.store(POISONED, Ordering::Release);
    }
}

// ─── Const constructor (CoreBackend only) ─────────────────────────────

impl<T, F: FnOnce() -> T> LazyLock<T, F> {
    /// Creates a new `LazyLock` with the given initializer.
    pub const fn new(init: F) -> Self {
        Self {
            state: core::sync::atomic::AtomicU8::new(UNINIT),
            value: core::cell::UnsafeCell::new(MaybeUninit::uninit()),
            init: core::cell::UnsafeCell::new(Some(init)),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<T, F: FnOnce() -> T, B: Backend> LazyLockInner<T, F, B> {
    /// Creates a new `LazyLockInner` using backend factory functions.
    pub fn new_with_backend(init: F) -> Self {
        Self {
            state: B::new_atomic_u8(UNINIT),
            value: B::new_unsafe_cell(MaybeUninit::uninit()),
            init: B::new_unsafe_cell(Some(init)),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<T, F: FnOnce() -> T, B: Backend> LazyLockInner<T, F, B> {
    /// Forces initialization if not already done, then returns a reference.
    fn force(&self) -> &T {
        match self.state.load(Ordering::Acquire) {
            READY => {
                // SAFETY: State is READY, so the value is fully initialized.
                return self.value.with(|ptr| unsafe { (*ptr).assume_init_ref() });
            }
            POISONED => panic!("LazyLock poisoned: init closure panicked"),
            UNINIT => {
                // Try to become the initializer.
                if self
                    .state
                    .compare_exchange(UNINIT, INITIALIZING, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
                {
                    // We won the race — initialize.
                    let guard = InitGuard::<B>::new(&self.state);
                    // SAFETY: We are the only thread in INITIALIZING state.
                    let init = self.init.with_mut(|ptr| unsafe { (*ptr).take().unwrap() });
                    let value = init();
                    self.value.with_mut(|ptr| unsafe {
                        (*ptr).write(value);
                    });
                    self.state.store(READY, Ordering::Release);
                    guard.defuse();
                    // SAFETY: We just wrote the value.
                    return self.value.with(|ptr| unsafe { (*ptr).assume_init_ref() });
                }
                // Another thread is initializing — fall through to spin.
            }
            _ => {} // INITIALIZING — spin below.
        }

        // Spin until the value is ready (or poisoned).
        loop {
            match self.state.load(Ordering::Acquire) {
                READY => break,
                POISONED => panic!("LazyLock poisoned: init closure panicked"),
                _ => B::spin_wait_hint(),
            }
        }
        // SAFETY: State is READY.
        self.value.with(|ptr| unsafe { (*ptr).assume_init_ref() })
    }
}

impl<T, F: FnOnce() -> T, B: Backend> Deref for LazyLockInner<T, F, B> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.force()
    }
}

impl<T, F, B: Backend> Drop for LazyLockInner<T, F, B> {
    fn drop(&mut self) {
        // Drop has exclusive access (&mut self), so Relaxed is sufficient.
        if self.state.load(Ordering::Relaxed) == READY {
            // SAFETY: State is READY, so the value was fully initialized.
            unsafe { self.value.with_mut(|ptr| (*ptr).assume_init_drop()) };
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn initializes_on_first_access() {
        let lazy = LazyLock::new(|| 42);
        assert_eq!(*lazy, 42);
    }

    #[test]
    fn init_called_once() {
        static CALL_COUNT: AtomicUsize = AtomicUsize::new(0);
        let lazy = LazyLock::new(|| {
            CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            99
        });
        assert_eq!(*lazy, 99);
        assert_eq!(*lazy, 99);
        assert_eq!(CALL_COUNT.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deref_returns_value() {
        let lazy = LazyLock::new(|| String::from("hello"));
        assert_eq!(&*lazy, "hello");
    }
}

#[cfg(loom)]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;

    use super::LazyLockInner;
    use crate::sync::atomic::{AtomicUsize, Ordering};
    use crate::sync::backend::LoomBackend;

    type LoomLazyLock<T, F> = LazyLockInner<T, F, LoomBackend>;

    /// Verify that when multiple threads race to initialize a LazyLock,
    /// the init closure runs exactly once and all threads see the same value.
    #[test]
    fn loom_lazy_init_race() {
        loom::model(|| {
            let init_count = Arc::new(AtomicUsize::new(0));
            let count_ref = init_count.clone();
            let lazy = Arc::new(LoomLazyLock::new_with_backend(move || {
                count_ref.fetch_add(1, Ordering::SeqCst);
                42usize
            }));

            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let lazy = lazy.clone();
                    thread::spawn(move || **lazy)
                })
                .collect();

            let mut results = Vec::new();
            for h in handles {
                results.push(h.join().unwrap());
            }

            // Init ran exactly once.
            assert_eq!(init_count.load(Ordering::SeqCst), 1);
            // All threads saw the same value.
            for &r in &results {
                assert_eq!(r, 42);
            }
        });
    }

    /// Verify Release/Acquire ordering: reader thread sees the initialized
    /// value after the writer thread completes initialization.
    #[test]
    fn loom_lazy_deref_after_init() {
        loom::model(|| {
            let lazy = Arc::new(LoomLazyLock::new_with_backend(|| 99usize));

            let l1 = lazy.clone();
            let t = thread::spawn(move || {
                // Force initialization.
                let _val: usize = **l1;
            });
            t.join().unwrap();

            // Reader sees the initialized value.
            assert_eq!(**lazy, 99);
        });
    }
}

#[cfg(shuttle)]
mod shuttle_tests {
    use shuttle::sync::Arc;
    use shuttle::thread;

    use super::LazyLockInner;
    use crate::sync::backend::ShuttleBackend;

    #[test]
    fn shuttle_init_once_under_contention() {
        shuttle::check_random(
            || {
                let init_count = Arc::new(shuttle::sync::atomic::AtomicUsize::new(0));
                let count_clone = init_count.clone();
                let lazy = Arc::new(LazyLockInner::<usize, _, ShuttleBackend>::new_with_backend(
                    move || {
                        count_clone.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
                        42usize
                    },
                ));

                // 4 threads race to initialize.
                let threads: Vec<_> = (0..4)
                    .map(|_| {
                        let lazy = lazy.clone();
                        thread::spawn(move || {
                            assert_eq!(**lazy, 42);
                        })
                    })
                    .collect();

                for t in threads {
                    t.join().unwrap();
                }

                // Init closure executed exactly once.
                assert_eq!(init_count.load(core::sync::atomic::Ordering::SeqCst), 1);
            },
            100,
        );
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Verify the state machine transitions: UNINIT -> READY after deref.
    #[kani::proof]
    fn lazy_state_machine_completeness() {
        let lazy = LazyLock::new(|| 42u32);
        // Before access, state is UNINIT.
        // After first access, should transition to READY.
        let val = *lazy;
        assert_eq!(val, 42);
        // Second access should also return the same value (still READY).
        assert_eq!(*lazy, 42);
    }

    /// Verify the init closure executes exactly once.
    #[kani::proof]
    fn lazy_init_once() {
        static CALL_COUNT: core::sync::atomic::AtomicUsize =
            core::sync::atomic::AtomicUsize::new(0);
        let lazy = LazyLock::new(|| {
            CALL_COUNT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
            99u32
        });
        let _ = *lazy;
        let _ = *lazy;
        assert_eq!(CALL_COUNT.load(core::sync::atomic::Ordering::SeqCst), 1);
    }
}
