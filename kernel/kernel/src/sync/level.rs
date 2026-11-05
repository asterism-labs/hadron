//! Lock levels: the order locks must be taken in.
//!
//! Every lock has a level, and the levels form one chain, declared in
//! [`levels`]. A thread may take a lock only if every lock it holds has an
//! earlier level. `lockdep` checks that at run time on every acquisition,
//! when the `LOCKDEP` config option is set.
//!
//! [`levels::MaySleep`] splits the chain. Sleeping locks come before it
//! and spin locks after it, and `Mutex::new` and `SpinLock::new` check
//! that at compile time. So taking locks in order never takes a sleeping
//! lock under a spin lock. `sched::might_sleep` catches the remaining
//! case: sleeping some other way while holding a spin lock.

/// A lock level.
pub(crate) trait Level {
    /// The level's name, for lock-order reports.
    const NAME: &'static str;
    /// The level's position in the chain, from 0. Locks must be taken in
    /// rising rank.
    const RANK: u16;
}

/// Declares the chain of lock levels, earliest first, separated by `=>`.
macro_rules! lock_levels {
    (@rank $rank:expr;) => {};
    (@rank $rank:expr; $(#[$meta:meta])* $level:ident $(=> $($rest:tt)*)?) => {
        $(#[$meta])*
        pub(crate) enum $level {}

        impl $crate::sync::level::Level for $level {
            const NAME: &'static str = stringify!($level);
            const RANK: u16 = $rank;
        }

        lock_levels!(@rank $rank + 1; $($($rest)*)?);
    };
    ($($chain:tt)*) => {
        lock_levels!(@rank 0; $($chain)*);
    };
}

/// The kernel's lock order, earliest first.
pub(crate) mod levels {
    lock_levels! {
        // Sleeping locks.
        /// A `Mutex` the kernel tests take first.
        KtestOuter
        => /// A `Mutex` the kernel tests take second.
           KtestInner
        // Sleeping locks come before this level, spin locks after it.
        => /// Marks where sleeping stops being allowed.
           MaySleep
        => /// The free list of kernel stacks.
           Stacks
        => /// The kernel's page tables.
           KernelTables
        => /// The frame allocator.
           Frames
        => /// A spin lock the kernel tests take.
           KtestSpin
        => /// The wakers inside a `WaitQueue`.
           WaitQueue
        => /// The queue of threads ready to run. `Waker`s take it, so it
           /// comes after every lock that a waker may be woken under.
           RunQueue
        => /// The kernel heap. The global allocator takes it from anywhere,
           /// so it is last and takes no other lock.
           Heap
    }
}
