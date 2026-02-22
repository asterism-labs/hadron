//! Counting semaphore.
//!
//! [`Semaphore`] limits concurrent access to a resource. Tasks acquire
//! permits before proceeding and release them when done.

use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::{Context, Poll};

use super::HeapWaitQueue;

/// A counting semaphore.
///
/// Controls access to a resource with a fixed number of permits.
/// Acquiring a permit decrements the count; releasing increments it.
///
/// # Example
///
/// ```ignore
/// static SEM: Semaphore = Semaphore::new(3); // 3 concurrent permits
///
/// async fn access_resource() {
///     let _permit = SEM.acquire().await;
///     // ... use the resource ...
///     // permit is released on drop
/// }
/// ```
pub struct Semaphore {
    permits: AtomicU32,
    waiters: HeapWaitQueue,
}

impl Semaphore {
    /// Creates a new semaphore with the given number of permits.
    pub const fn new(permits: u32) -> Self {
        Self {
            permits: AtomicU32::new(permits),
            waiters: HeapWaitQueue::new(),
        }
    }

    /// Asynchronously acquires a permit.
    ///
    /// If no permits are available, the current task yields until one
    /// is released.
    pub fn acquire(&self) -> SemaphoreAcquireFuture<'_> {
        SemaphoreAcquireFuture { sem: self }
    }

    /// Tries to acquire a permit without blocking.
    ///
    /// Returns `Some(permit)` if a permit was available, `None` otherwise.
    pub fn try_acquire(&self) -> Option<SemaphorePermit<'_>> {
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
                return Some(SemaphorePermit { sem: self });
            }
        }
    }

    /// Returns the number of currently available permits.
    pub fn available_permits(&self) -> u32 {
        self.permits.load(Ordering::Relaxed)
    }

    /// Releases a permit back to the semaphore.
    ///
    /// Usually called automatically by [`SemaphorePermit::drop`].
    fn release(&self) {
        self.permits.fetch_add(1, Ordering::Release);
        self.waiters.wake_one();
    }
}

/// Future returned by [`Semaphore::acquire`].
pub struct SemaphoreAcquireFuture<'a> {
    sem: &'a Semaphore,
}

impl<'a> Future for SemaphoreAcquireFuture<'a> {
    type Output = SemaphorePermit<'a>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Fast path: try to acquire directly.
        if let Some(permit) = self.sem.try_acquire() {
            return Poll::Ready(permit);
        }

        // Register waker before retry.
        self.sem.waiters.register_waker(cx.waker());

        // Retry after registration.
        if let Some(permit) = self.sem.try_acquire() {
            return Poll::Ready(permit);
        }

        Poll::Pending
    }
}

/// RAII permit that releases back to the [`Semaphore`] on drop.
pub struct SemaphorePermit<'a> {
    sem: &'a Semaphore,
}

impl Drop for SemaphorePermit<'_> {
    fn drop(&mut self) {
        self.sem.release();
    }
}

#[cfg(test)]
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
        // Permit dropped — should be available again.
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
