//! Test context and async barrier for concurrent instanced tests.

use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::{Context, Poll};

/// Context provided to instanced test functions.
///
/// Each concurrent instance receives its own `TestContext` with a unique
/// `instance_id` and a shared barrier for synchronization.
pub struct TestContext {
    /// Zero-based instance identifier within the test.
    pub instance_id: u32,
    /// Total number of concurrent instances.
    pub instance_count: u32,
    barrier_inner: Arc<AsyncBarrier>,
}

impl TestContext {
    /// Creates a new test context for one instance.
    pub fn new(instance_id: u32, instance_count: u32, barrier: Arc<AsyncBarrier>) -> Self {
        Self {
            instance_id,
            instance_count,
            barrier_inner: barrier,
        }
    }

    /// Waits until all instances have reached this barrier point.
    ///
    /// This is a cooperative yield — the executor runs other tasks while
    /// waiting for all instances to arrive.
    pub async fn barrier(&self) {
        self.barrier_inner.wait().await;
    }
}

/// A simple async barrier for synchronizing concurrent test instances.
///
/// Each call to [`wait()`](AsyncBarrier::wait) increments an arrival counter.
/// When the counter reaches the configured total, all waiters proceed.
pub struct AsyncBarrier {
    total: u32,
    count: AtomicU32,
}

impl AsyncBarrier {
    /// Creates a new barrier that releases after `total` parties arrive.
    pub fn new(total: u32) -> Self {
        Self {
            total,
            count: AtomicU32::new(0),
        }
    }

    /// Returns a future that completes when all parties have called `wait`.
    pub fn wait(&self) -> BarrierWait<'_> {
        BarrierWait {
            barrier: self,
            registered: false,
        }
    }
}

/// Future returned by [`AsyncBarrier::wait()`].
pub struct BarrierWait<'a> {
    barrier: &'a AsyncBarrier,
    registered: bool,
}

impl Future for BarrierWait<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if !self.registered {
            self.registered = true;
            self.barrier.count.fetch_add(1, Ordering::Release);
        }

        if self.barrier.count.load(Ordering::Acquire) >= self.barrier.total {
            Poll::Ready(())
        } else {
            // Re-schedule so the executor polls us again after running other tasks.
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
