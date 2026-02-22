//! Timer-based waker registry.
//!
//! Sleeping tasks register their waker and deadline here. The timer
//! interrupt handler calls [`wake_expired`] each tick to wake tasks
//! whose deadline has passed.

use alloc::collections::BinaryHeap;
use core::cmp::{Ordering, Reverse};
use core::task::Waker;

use planck_noalloc::vec::ArrayVec;

use crate::sync::IrqSpinLock;

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
