//! Futex — fast userspace mutex primitives.
//!
//! Provides `FUTEX_WAIT` (sleep if `*addr == val`) and `FUTEX_WAKE`
//! (wake waiters sleeping on `addr`). Used by pthreads mutexes and
//! condition variables.
//!
//! Addresses are resolved to physical addresses so that shared memory
//! mappings work correctly across processes.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::task::Waker;

use crate::sync::SpinLock;

/// Maximum concurrent futex addresses tracked.
const MAX_FUTEX_ENTRIES: usize = 1024;

/// Global futex table mapping user virtual addresses to wait queues.
///
/// Uses virtual addresses for simplicity (physical address resolution
/// can be added later for shared-memory futexes across processes).
static FUTEX_TABLE: SpinLock<FutexTable> = SpinLock::named("futex_table", FutexTable::new());

/// Futex wait queue table.
struct FutexTable {
    /// Map from user virtual address to list of waiting wakers.
    waiters: BTreeMap<usize, Vec<Waker>>,
}

impl FutexTable {
    const fn new() -> Self {
        Self {
            waiters: BTreeMap::new(),
        }
    }

    /// Register a waker for the given address. Returns false if table is full.
    fn register(&mut self, addr: usize, waker: Waker) -> bool {
        // Check capacity before taking a mutable borrow via entry().
        if self.waiters.len() >= MAX_FUTEX_ENTRIES && !self.waiters.contains_key(&addr) {
            return false;
        }
        self.waiters
            .entry(addr)
            .or_insert_with(Vec::new)
            .push(waker);
        true
    }

    /// Wake up to `count` waiters on the given address. Returns number woken.
    fn wake(&mut self, addr: usize, count: usize) -> usize {
        let Some(waiters) = self.waiters.get_mut(&addr) else {
            return 0;
        };
        let to_wake = count.min(waiters.len());
        for waker in waiters.drain(..to_wake) {
            waker.wake();
        }
        // Clean up empty entries.
        if waiters.is_empty() {
            self.waiters.remove(&addr);
        }
        to_wake
    }
}

/// FUTEX_WAIT: if `*addr == expected`, register waker and return Pending.
///
/// Called from the trap handler's async context. Returns `true` if the
/// condition was met and we should sleep, `false` if the value changed.
pub fn futex_wait_check(addr: usize, expected: u32, waker: &Waker) -> bool {
    // SAFETY: addr points to user memory, caller has switched to user CR3.
    let current = unsafe { core::ptr::read_volatile(addr as *const u32) };
    if current != expected {
        return false; // Value changed, don't sleep.
    }

    let mut table = FUTEX_TABLE.lock();
    table.register(addr, waker.clone());
    true
}

/// FUTEX_WAKE: wake up to `count` threads sleeping on `addr`.
///
/// Returns the number of threads actually woken.
pub fn futex_wake(addr: usize, count: usize) -> usize {
    let mut table = FUTEX_TABLE.lock();
    table.wake(addr, count)
}
