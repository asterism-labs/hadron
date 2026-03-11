//! Global futex table for userspace synchronization.
//!
//! Provides the kernel-side of the futex mechanism. A futex is identified
//! by a `(process_koid, virtual_address)` pair. [`futex_wait`] registers a
//! waker that is woken by a corresponding [`futex_wake`].

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::collections::VecDeque;
use core::task::Waker;

use hadron_core::sync::IrqSpinLock;

/// Global futex wait queue, keyed by `(process_koid, virtual_address)`.
static FUTEX_TABLE: IrqSpinLock<BTreeMap<(u64, u64), VecDeque<Waker>>> =
    IrqSpinLock::leveled("FUTEX_TABLE", 11, BTreeMap::new());

/// Register a waker to be woken by a matching `futex_wake`.
pub fn futex_wait(koid: u64, addr: u64, waker: Waker) {
    FUTEX_TABLE
        .lock()
        .entry((koid, addr))
        .or_default()
        .push_back(waker);
}

/// Wake up to `count` waiters on the given futex address.
///
/// Returns the number of waiters actually woken.
pub fn futex_wake(koid: u64, addr: u64, count: u32) -> usize {
    let mut table = FUTEX_TABLE.lock();
    let Some(waiters) = table.get_mut(&(koid, addr)) else {
        return 0;
    };

    let wake_count = (count as usize).min(waiters.len());
    let to_wake: VecDeque<Waker> = waiters.drain(..wake_count).collect();

    // Remove the entry if no waiters remain.
    if waiters.is_empty() {
        table.remove(&(koid, addr));
    }

    // Drop lock before waking to avoid holding FUTEX_TABLE during
    // executor enqueue.
    drop(table);

    for waker in to_wake {
        waker.wake();
    }

    wake_count
}
