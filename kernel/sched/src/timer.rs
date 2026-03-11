//! Timer-based waker registry.
//!
//! Sleeping tasks register their waker and deadline here. The timer
//! interrupt handler calls [`wake_expired`] each tick to wake tasks
//! whose deadline has passed.

use alloc::collections::BinaryHeap;
use core::cmp::{Ordering, Reverse};
use core::task::Waker;

use planck_noalloc::vec::ArrayVec;

use hadron_core::sync::IrqSpinLock;

struct SleepEntry {
    deadline: u64,
    waker: Waker,
}

impl PartialEq for SleepEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline
    }
}

impl Eq for SleepEntry {}

impl PartialOrd for SleepEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SleepEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.deadline.cmp(&other.deadline)
    }
}

static SLEEP_QUEUE: IrqSpinLock<BinaryHeap<Reverse<SleepEntry>>> =
    IrqSpinLock::leveled("SLEEP_QUEUE", 12, BinaryHeap::new());

/// Registers a waker to be called when `deadline` tick is reached.
pub fn register_sleep_waker(deadline: u64, waker: Waker) {
    SLEEP_QUEUE
        .lock()
        .push(Reverse(SleepEntry { deadline, waker }));
}

/// Maximum wakers drained per tick. If more are expired, they are deferred
/// to the next tick (1 ms later). Keeps the ISR bounded and stack-allocated.
const WAKE_BATCH_SIZE: usize = 32;

/// Wakes all tasks whose sleep deadline has passed.
///
/// Called from the timer interrupt handler on every tick. Drains expired
/// entries into a stack-allocated batch, drops the SLEEP_QUEUE lock, then
/// wakes outside the lock to avoid holding SLEEP_QUEUE while calling into
/// the executor's ready queues.
pub fn wake_expired(current_tick: u64) {
    let mut batch = ArrayVec::<Waker, WAKE_BATCH_SIZE>::new();

    {
        let mut queue = SLEEP_QUEUE.lock();
        while batch.len() < WAKE_BATCH_SIZE {
            match queue.peek() {
                Some(entry) if entry.0.deadline <= current_tick => {
                    let entry = queue.pop().unwrap();
                    batch.push(entry.0.waker);
                }
                _ => break,
            }
        }
        // Lock dropped here — remaining expired entries (if batch was full)
        // will be picked up on the next tick (1 ms later).
    }

    while let Some(waker) = batch.pop() {
        waker.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::task::{RawWaker, RawWakerVTable, Waker};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    /// Creates a [`Waker`] that increments a counter each time it is woken.
    fn counting_waker() -> (Waker, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let data = Arc::into_raw(counter.clone()) as *const ();

        unsafe fn clone(data: *const ()) -> RawWaker {
            let arc = unsafe { Arc::from_raw(data.cast::<AtomicUsize>()) };
            let cloned = arc.clone();
            let _ = Arc::into_raw(arc);
            RawWaker::new(Arc::into_raw(cloned).cast::<()>(), &VTABLE)
        }

        unsafe fn wake(data: *const ()) {
            let arc = unsafe { Arc::from_raw(data.cast::<AtomicUsize>()) };
            arc.fetch_add(1, AtomicOrdering::SeqCst);
        }

        unsafe fn wake_by_ref(data: *const ()) {
            let arc = unsafe { Arc::from_raw(data.cast::<AtomicUsize>()) };
            arc.fetch_add(1, AtomicOrdering::SeqCst);
            let _ = Arc::into_raw(arc);
        }

        unsafe fn drop_waker(data: *const ()) {
            unsafe { drop(Arc::from_raw(data.cast::<AtomicUsize>())) };
        }

        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);

        let waker = unsafe { Waker::from_raw(RawWaker::new(data, &VTABLE)) };
        (waker, counter)
    }

    /// Drains all entries from the global sleep queue so tests don't leak state.
    fn drain_queue() {
        wake_expired(u64::MAX);
    }

    #[test]
    fn register_and_wake_expired() {
        drain_queue();

        let (waker, count) = counting_waker();
        register_sleep_waker(100, waker);

        wake_expired(99);
        assert_eq!(count.load(AtomicOrdering::Relaxed), 0);

        wake_expired(100);
        assert_eq!(count.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn multiple_wakers_ordered_by_deadline() {
        drain_queue();

        let (w1, c1) = counting_waker();
        let (w2, c2) = counting_waker();
        let (w3, c3) = counting_waker();

        register_sleep_waker(10, w1);
        register_sleep_waker(20, w2);
        register_sleep_waker(30, w3);

        wake_expired(15);
        assert_eq!(c1.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(c2.load(AtomicOrdering::Relaxed), 0);
        assert_eq!(c3.load(AtomicOrdering::Relaxed), 0);

        wake_expired(25);
        assert_eq!(c2.load(AtomicOrdering::Relaxed), 1);
        assert_eq!(c3.load(AtomicOrdering::Relaxed), 0);

        wake_expired(35);
        assert_eq!(c3.load(AtomicOrdering::Relaxed), 1);
    }

    #[test]
    fn wake_batch_overflow() {
        drain_queue();

        let wakers: Vec<_> = (0..40).map(|_| counting_waker()).collect();
        for (w, _) in &wakers {
            register_sleep_waker(1, w.clone());
        }

        // First call wakes batch of 32, second call wakes remaining 8.
        wake_expired(1);
        wake_expired(1);

        for (_, count) in &wakers {
            assert_eq!(count.load(AtomicOrdering::Relaxed), 1);
        }
    }

    #[test]
    fn expired_on_registration() {
        drain_queue();

        let (waker, count) = counting_waker();
        register_sleep_waker(0, waker);

        wake_expired(1);
        assert_eq!(count.load(AtomicOrdering::Relaxed), 1);
    }
}
