//! Run-time lock-order checks, on when the `LOCKDEP` config option is set.
//!
//! Each acquisition records the lock's rank, name, address and caller on a
//! held-lock stack:
//!
//! - spin locks on the CPU's stack, since a spin lock is never held across
//!   a context switch;
//! - sleeping locks on the thread's stack, since a thread can sleep while
//!   holding one.
//!
//! Taking a lock whose rank is not above every held lock's rank panics and
//! names both locks and where each was taken. Ranks are fixed, so one run
//! that takes two locks out of order is enough: the reverse order never
//! has to happen. That also catches two locks at the same level, and an
//! exception handler that takes a lock its interrupted code holds.

use core::cell::Cell;
use core::panic::Location;

use super::level::Level;
use crate::percpu::{self, IrqGuard};

/// The most locks one CPU or one thread may hold at once.
const MAX_HELD: usize = 16;

/// One held lock.
#[derive(Clone, Copy)]
struct Held {
    rank: u16,
    name: &'static str,
    addr: usize,
    site: &'static Location<'static>,
}

/// A stack of held locks. Only its owner, a CPU or a thread, uses it.
pub(crate) struct HeldLocks {
    entries: [Cell<Option<Held>>; MAX_HELD],
    len: Cell<usize>,
}

impl HeldLocks {
    /// An empty stack.
    pub(crate) const fn new() -> Self {
        Self {
            entries: [const { Cell::new(None) }; MAX_HELD],
            len: Cell::new(0),
        }
    }

    /// The most recently taken lock that is still held.
    fn top(&self) -> Option<Held> {
        self.len
            .get()
            .checked_sub(1)
            .and_then(|i| self.entries[i].get())
    }

    fn push(&self, held: Held) {
        let len = self.len.get();
        assert!(len < MAX_HELD, "more than {MAX_HELD} locks held at once");
        self.entries[len].set(Some(held));
        self.len.set(len + 1);
    }

    /// Removes the entry for `addr`. Locks may be released in any order,
    /// so it need not be on top.
    fn remove(&self, addr: usize) {
        let len = self.len.get();
        let found = (0..len)
            .rev()
            .find(|&i| self.entries[i].get().is_some_and(|held| held.addr == addr));
        let Some(index) = found else {
            panic!("lockdep: released a lock at {addr:#x} that was not recorded");
        };
        for i in index..len - 1 {
            self.entries[i].set(self.entries[i + 1].get());
        }
        self.entries[len - 1].set(None);
        self.len.set(len - 1);
    }
}

/// Which stack an acquisition goes on.
#[derive(Clone, Copy)]
pub(super) enum Kind {
    /// A spin lock, recorded on the CPU.
    Spin,
    /// A sleeping lock, recorded on the thread.
    Sleeping,
}

/// Checks that a lock at level `L` may be taken now, and records it.
///
/// # Panics
///
/// Panics if a held lock's rank is not below `L`'s.
pub(super) fn acquire<L: Level>(kind: Kind, addr: usize, site: &'static Location<'static>) {
    if !hadron_config::LOCKDEP {
        return;
    }
    let irq = IrqGuard::new();
    let cpu = percpu::current(&irq);
    let thread = cpu.current_thread();
    let new = Held {
        rank: L::RANK,
        name: L::NAME,
        addr,
        site,
    };
    let held = [Some(&cpu.held), thread.map(crate::sched::Thread::held)];
    for top in held.into_iter().flatten().filter_map(HeldLocks::top) {
        assert!(
            top.rank < new.rank,
            "lock order: taking {} at {} while holding {} taken at {}",
            new.name,
            new.site,
            top.name,
            top.site
        );
    }
    match (kind, thread) {
        (Kind::Spin, _) => cpu.held.push(new),
        (Kind::Sleeping, Some(thread)) => thread.held().push(new),
        (Kind::Sleeping, None) => panic!(
            "{} is a sleeping lock, taken before threads exist",
            new.name
        ),
    }
}

/// Forgets the lock at `addr`, which is being released.
pub(super) fn release(kind: Kind, addr: usize) {
    if !hadron_config::LOCKDEP {
        return;
    }
    let irq = IrqGuard::new();
    let cpu = percpu::current(&irq);
    match kind {
        Kind::Spin => cpu.held.remove(addr),
        Kind::Sleeping => cpu
            .current_thread()
            .expect("sleeping locks are held by threads")
            .held()
            .remove(addr),
    }
}
