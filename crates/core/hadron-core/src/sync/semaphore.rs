//! Counting semaphore.
//!
//! [`Semaphore`] limits concurrent access to a resource. Tasks acquire
//! permits before proceeding and release them when done.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::Ordering;
use core::task::{Context, Poll};

use super::backend::{AtomicIntOps, CoreBackend, IrqBackend};
use super::heap_waitqueue::{HeapWaitQueue, HeapWaitQueueInner};

// ─── Type aliases ─────────────────────────────────────────────────────

/// A counting semaphore.
///
/// Controls access to a resource with a fixed number of permits.
pub type Semaphore = SemaphoreInner<CoreBackend>;

/// Future returned by [`Semaphore::acquire`].
pub type SemaphoreAcquireFuture<'a> = SemaphoreAcquireFutureInner<'a, CoreBackend>;

/// RAII permit that releases back to the [`Semaphore`] on drop.
pub type SemaphorePermit<'a> = SemaphorePermitInner<'a, CoreBackend>;

// ─── Generic inner type ───────────────────────────────────────────────

/// Backend-generic counting semaphore.
pub struct SemaphoreInner<B: IrqBackend> {
    permits: B::AtomicU32,
    waiters: HeapWaitQueueInner<B>,
}

// ─── Const constructor (CoreBackend only) ─────────────────────────────

impl Semaphore {
    /// Creates a new semaphore with the given number of permits.
    pub const fn new(permits: u32) -> Self {
        Self {
            permits: core::sync::atomic::AtomicU32::new(permits),
            waiters: HeapWaitQueue::new(),
        }
    }
}

// ─── Generic non-const constructor ────────────────────────────────────

impl<B: IrqBackend> SemaphoreInner<B> {
    /// Creates a new semaphore using backend factory functions.
    pub fn new_with_backend(permits: u32) -> Self {
        Self {
            permits: B::new_atomic_u32(permits),
            waiters: HeapWaitQueueInner::new_with_backend(),
        }
    }
}

// ─── Algorithm (generic over B) ───────────────────────────────────────

impl<B: IrqBackend> SemaphoreInner<B> {
    /// Asynchronously acquires a permit.
    pub fn acquire(&self) -> SemaphoreAcquireFutureInner<'_, B> {
        SemaphoreAcquireFutureInner { sem: self }
    }

    /// Tries to acquire a permit without blocking.
    pub fn try_acquire(&self) -> Option<SemaphorePermitInner<'_, B>> {
        loop {
            let current = self.permits.load(Ordering::Relaxed);
            if current == 0 {
                return None;
            }
            if self
                .permits
                .compare_exchange_weak(current, current - 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Some(SemaphorePermitInner { sem: self });
            }
        }
    }

    /// Returns the number of currently available permits.
    pub fn available_permits(&self) -> u32 {
        self.permits.load(Ordering::Relaxed)
    }

    /// Releases a permit back to the semaphore.
    fn release(&self) {
        self.permits.fetch_add(1, Ordering::Release);
        self.waiters.wake_one();
    }
}

// ─── Acquire future ───────────────────────────────────────────────────

/// Future returned by [`SemaphoreInner::acquire`].
pub struct SemaphoreAcquireFutureInner<'a, B: IrqBackend> {
    sem: &'a SemaphoreInner<B>,
}

impl<'a, B: IrqBackend> Future for SemaphoreAcquireFutureInner<'a, B> {
    type Output = SemaphorePermitInner<'a, B>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(permit) = self.sem.try_acquire() {
            return Poll::Ready(permit);
        }

        self.sem.waiters.register_waker(cx.waker());

        if let Some(permit) = self.sem.try_acquire() {
            return Poll::Ready(permit);
        }

        Poll::Pending
    }
}

// ─── Permit ───────────────────────────────────────────────────────────

/// RAII permit that releases back to the [`SemaphoreInner`] on drop.
pub struct SemaphorePermitInner<'a, B: IrqBackend> {
    sem: &'a SemaphoreInner<B>,
}

impl<B: IrqBackend> Drop for SemaphorePermitInner<'_, B> {
    fn drop(&mut self) {
        self.sem.release();
    }
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;

    #[test]
    fn try_acquire_succeeds() {
        let sem = Semaphore::new(2);
        let p1 = sem.try_acquire();
        assert!(p1.is_some());
        assert_eq!(sem.available_permits(), 1);
    }

    #[test]
    fn try_acquire_exhausts_permits() {
        let sem = Semaphore::new(2);
        let _p1 = sem.try_acquire().unwrap();
        let _p2 = sem.try_acquire().unwrap();
        assert!(sem.try_acquire().is_none());
        assert_eq!(sem.available_permits(), 0);
    }

    #[test]
    fn permit_drop_releases() {
        let sem = Semaphore::new(1);
        {
            let _p = sem.try_acquire().unwrap();
            assert_eq!(sem.available_permits(), 0);
        }
        assert_eq!(sem.available_permits(), 1);
        assert!(sem.try_acquire().is_some());
    }

    #[test]
    fn zero_permits() {
        let sem = Semaphore::new(0);
        assert!(sem.try_acquire().is_none());
    }

    #[test]
    fn multiple_acquire_release_cycles() {
        let sem = Semaphore::new(3);
        for _ in 0..10 {
            let _p1 = sem.try_acquire().unwrap();
            let _p2 = sem.try_acquire().unwrap();
            let _p3 = sem.try_acquire().unwrap();
            assert!(sem.try_acquire().is_none());
        }
        assert_eq!(sem.available_permits(), 3);
    }

    #[test]
    fn acquire_future_ready_when_available() {
        use crate::sync::test_waker::noop_waker;
        let sem = Semaphore::new(1);
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = sem.acquire();
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Ready(_)));
    }

    #[test]
    fn acquire_future_pending_when_exhausted() {
        use crate::sync::test_waker::noop_waker;
        let sem = Semaphore::new(1);
        let _p = sem.try_acquire().unwrap();
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut fut = sem.acquire();
        let result = Pin::new(&mut fut).poll(&mut cx);
        assert!(matches!(result, Poll::Pending));
    }
}

#[cfg(loom)]
mod loom_tests {
    use loom::sync::Arc;
    use loom::thread;

    use super::SemaphoreInner;
    use crate::sync::atomic::Ordering;
    use crate::sync::backend::LoomBackend;

    type LoomSemaphore = SemaphoreInner<LoomBackend>;

    #[test]
    fn loom_semaphore_permit_exhaustion() {
        loom::model(|| {
            let sem = Arc::new(LoomSemaphore::new_with_backend(1));
            let in_critical = Arc::new(crate::sync::atomic::AtomicUsize::new(0));

            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let sem = sem.clone();
                    let crit = in_critical.clone();
                    thread::spawn(move || {
                        let _permit = loop {
                            if let Some(p) = sem.try_acquire() {
                                break p;
                            }
                            loom::thread::yield_now();
                        };

                        let prev = crit.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(prev, 0, "two threads in critical section");
                        crit.fetch_sub(1, Ordering::SeqCst);
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }
        });
    }

    #[test]
    fn loom_semaphore_release_wakes_waiter() {
        use core::future::Future;
        use core::pin::Pin;
        use core::task::Context;

        loom::model(|| {
            let sem = Arc::new(LoomSemaphore::new_with_backend(1));

            let s1 = sem.clone();
            let s2 = sem.clone();

            let t1 = thread::spawn(move || {
                let _p = loop {
                    if let Some(p) = s1.try_acquire() {
                        break p;
                    }
                    loom::thread::yield_now();
                };
            });

            let t2 = thread::spawn(move || {
                let waker = crate::sync::test_waker::noop_waker();
                let mut cx = Context::from_waker(&waker);
                let mut fut = s2.acquire();
                loop {
                    match Pin::new(&mut fut).poll(&mut cx) {
                        core::task::Poll::Ready(_permit) => break,
                        core::task::Poll::Pending => loom::thread::yield_now(),
                    }
                }
            });

            t1.join().unwrap();
            t2.join().unwrap();

            assert_eq!(sem.available_permits(), 1);
        });
    }

    #[test]
    fn loom_semaphore_no_permit_leak() {
        loom::model(|| {
            let sem = Arc::new(LoomSemaphore::new_with_backend(2));

            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let sem = sem.clone();
                    thread::spawn(move || {
                        let _p = loop {
                            if let Some(p) = sem.try_acquire() {
                                break p;
                            }
                            loom::thread::yield_now();
                        };
                    })
                })
                .collect();

            for h in handles {
                h.join().unwrap();
            }

            assert_eq!(sem.available_permits(), 2);
        });
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Verify permits never underflow: acquire + release preserves count.
    #[kani::proof]
    fn semaphore_permits_non_negative() {
        let n: u32 = kani::any();
        kani::assume(n > 0 && n <= 8);
        let sem = Semaphore::new(n);
        assert_eq!(sem.available_permits(), n);

        let p = sem.try_acquire();
        assert!(p.is_some());
        assert_eq!(sem.available_permits(), n - 1);
        drop(p);
        assert_eq!(sem.available_permits(), n);
    }

    /// Verify release always increases available permits by one.
    #[kani::proof]
    fn semaphore_release_monotonic() {
        let sem = Semaphore::new(1);
        let p = sem.try_acquire().unwrap();
        assert_eq!(sem.available_permits(), 0);
        drop(p);
        assert_eq!(sem.available_permits(), 1);
    }
}
