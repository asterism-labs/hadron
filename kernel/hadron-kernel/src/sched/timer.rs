//! Timer-based waker registry.
//!
//! Sleeping tasks register their waker and deadline here. The timer
//! interrupt handler calls [`wake_expired`] each tick to wake tasks
//! whose deadline has passed.

use alloc::collections::BinaryHeap;
use core::cmp::{Ordering, Reverse};
use core::task::Waker;

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
    IrqSpinLock::new(BinaryHeap::new());

/// Registers a waker to be called when `deadline` tick is reached.
pub fn register_sleep_waker(deadline: u64, waker: Waker) {
    SLEEP_QUEUE
        .lock()
        .push(Reverse(SleepEntry { deadline, waker }));
}

/// Wakes all tasks whose sleep deadline has passed.
///
/// Called from the timer interrupt handler on every tick.
pub fn wake_expired(current_tick: u64) {
    let mut queue = SLEEP_QUEUE.lock();
    while let Some(entry) = queue.peek() {
        if entry.0.deadline <= current_tick {
            let entry = queue.pop().unwrap();
            entry.0.waker.wake();
        } else {
            break;
        }
    }
}
