//! Lazy initialization primitive for `no_std`.
//!
//! Provides [`LazyLock`], a `no_std` equivalent of `std::sync::LazyLock`
//! that initializes a value on first access using a spin-based state machine.

use core::mem::MaybeUninit;
use core::ops::Deref;

use super::atomic::{AtomicU8, Ordering};
use super::cell::UnsafeCell;

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
pub struct LazyLock<T, F = fn() -> T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
    init: UnsafeCell<Option<F>>,
}

// SAFETY: The atomic state machine ensures that the value is fully initialized
// before any thread can read it, and that the init closure is consumed exactly
// once.
unsafe impl<T: Send + Sync, F: Send> Send for LazyLock<T, F> {}
unsafe impl<T: Send + Sync, F: Send> Sync for LazyLock<T, F> {}

/// Guard that poisons the `LazyLock` if dropped without completing init.
///
/// On successful initialization, the caller calls [`InitGuard::defuse`] to
/// prevent poisoning. If the init closure panics (unwind), the `Drop` impl
/// transitions the state to `POISONED`.
struct InitGuard<'a> {
    state: &'a AtomicU8,
}

impl<'a> InitGuard<'a> {
    fn new(state: &'a AtomicU8) -> Self {
        Self { state }
    }

    /// Disarm the guard after successful initialization.
    fn defuse(self) {
        core::mem::forget(self);
    }
}

impl Drop for InitGuard<'_> {
    fn drop(&mut self) {
        self.state.store(POISONED, Ordering::Release);
    }
}

impl<T, F: FnOnce() -> T> LazyLock<T, F> {
    maybe_const_fn! {
        /// Creates a new `LazyLock` with the given initializer.
        pub fn new(init: F) -> Self {
            Self {
                state: AtomicU8::new(UNINIT),
                value: UnsafeCell::new(MaybeUninit::uninit()),
                init: UnsafeCell::new(Some(init)),
            }
        }
    }

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
                    let guard = InitGuard::new(&self.state);
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
        //
        // Safety argument for liveness: with `panic = abort` (our kernel
        // target), a panic in the init closure halts the entire kernel, so
        // we can never get stuck in INITIALIZING. The POISONED check below
        // is defense-in-depth for non-abort environments (unit tests).
        loop {
            match self.state.load(Ordering::Acquire) {
                READY => break,
                POISONED => panic!("LazyLock poisoned: init closure panicked"),
                _ => super::spin_wait_hint(),
            }
        }
        // SAFETY: State is READY.
        self.value.with(|ptr| unsafe { (*ptr).assume_init_ref() })
    }
}

impl<T, F: FnOnce() -> T> Deref for LazyLock<T, F> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.force()
    }
}

impl<T, F> Drop for LazyLock<T, F> {
    fn drop(&mut self) {
        // Drop has exclusive access (&mut self), so Relaxed is sufficient.
        if self.state.load(Ordering::Relaxed) == READY {
            // SAFETY: State is READY, so the value was fully initialized.
            unsafe { self.value.with_mut(|ptr| (*ptr).assume_init_drop()) };
        }
    }
}

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

    use super::super::atomic::{AtomicUsize, Ordering};
    use super::LazyLock;

    /// Verify that when multiple threads race to initialize a LazyLock,
    /// the init closure runs exactly once and all threads see the same value.
    #[test]
    fn loom_lazy_init_race() {
        loom::model(|| {
            let init_count = Arc::new(AtomicUsize::new(0));
            let count_ref = init_count.clone();
            let lazy = Arc::new(LazyLock::new(move || {
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
            let lazy = Arc::new(LazyLock::new(|| 99usize));

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
