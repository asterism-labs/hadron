//! Global futex table for userspace synchronization.
//!
//! Thin wrapper around [`hadron_sched::futex::FutexTableInner`] with a
//! global `IrqSpinLock`. The core logic lives in `hadron-sched` so it
//! can be tested on the host.

use core::task::Waker;

use hadron_core::sync::IrqSpinLock;
use hadron_sched::futex::FutexTableInner;

/// Global futex wait queue, keyed by `(process_koid, virtual_address)`.
static FUTEX_TABLE: IrqSpinLock<FutexTableInner> =
    IrqSpinLock::leveled("FUTEX_TABLE", 11, FutexTableInner::new());

/// Register a waker to be woken by a matching [`futex_wake`].
pub fn futex_wait(koid: u64, addr: u64, waker: Waker) {
    FUTEX_TABLE.lock().wait(koid, addr, waker);
}

/// Wake up to `count` waiters on the given futex address.
///
/// Returns the number of waiters actually woken.
pub fn futex_wake(koid: u64, addr: u64, count: u32) -> usize {
    let mut table = FUTEX_TABLE.lock();
    let (wake_count, to_wake) = table.wake(koid, addr, count);

    // Drop lock before waking to avoid holding FUTEX_TABLE during
    // executor enqueue.
    drop(table);

    for waker in to_wake {
        waker.wake();
    }

    wake_count
}

// ── Kernel integration tests ────────────────────────────────────────

#[cfg(ktest)]
mod ktest {
    use hadron_ktest::kernel_test;

    /// Verifies that a futex waiter can be woken from the same task.
    #[kernel_test(stage = "with_executor")]
    async fn test_futex_wake_self() {
        use core::future::Future;
        use core::pin::Pin;
        use core::task::{Context, Poll};

        struct FutexWaitOnce {
            registered: bool,
        }

        impl Future for FutexWaitOnce {
            type Output = ();

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.registered {
                    return Poll::Ready(());
                }
                self.registered = true;
                super::futex_wait(999, 0x1000, cx.waker().clone());
                // Immediately wake to unblock.
                let woken = super::futex_wake(999, 0x1000, 1);
                assert_eq!(woken, 1);
                Poll::Pending
            }
        }

        FutexWaitOnce { registered: false }.await;
    }
}
