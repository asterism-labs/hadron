//! Lock levels and the tokens that check lock order at compile time.
//!
//! Every lock has a level, and the levels form one chain, declared in
//! [`levels`]. Code that takes a lock passes a [`Ctx`] token for the level
//! it holds now, and the lock accepts it only if that level comes earlier
//! in the chain. Taking the lock yields a token for the lock's own level
//! and borrows the old one, so the old one cannot be used again until the
//! lock is released.
//!
//! The order also checks context. Interrupt handlers will start from
//! [`levels::IrqRoot`], below every sleeping lock, so they cannot take one.
//! Sleeping needs a token from before [`levels::MaySleep`], and every spin
//! lock comes after it, so nothing sleeps while holding a spin lock.
//!
//! Each level has one parent, so the order is a chain (or, in general, a
//! tree) rather than any DAG. That keeps [`lock_levels!`] a `macro_rules`
//! macro: it gives a level the parent's ancestors through a blanket impl,
//! which works only with a single parent.
//!
//! Some locks are reached through standard traits that cannot take a
//! token: the heap through `GlobalAlloc`, the run queue through `Waker`.
//! Those use `lock_without_ctx`, and only the run-time checker in
//! `lockdep` sees them.

use core::marker::PhantomData;

/// A lock level.
pub(crate) trait Level {
    /// The level's name, for lock-order reports.
    const NAME: &'static str;
    /// The level's depth in the chain. The run-time checker requires
    /// ranks to rise as locks are taken.
    const RANK: u16;
}

/// `Self` comes after `A` in the lock order.
#[diagnostic::on_unimplemented(
    message = "a lock is taken, or a thread sleeps, out of lock order",
    label = "not allowed with the locks held here",
    note = "the notes below name the level that does not come before the other",
    note = "the lock order is declared in `sync::level::levels`"
)]
pub(crate) trait LockAfter<A> {}

/// `Self` comes before `B` in the lock order.
#[diagnostic::on_unimplemented(
    message = "lock order: `{Self}` does not come before `{B}`",
    note = "the lock order is declared in `sync::level::levels`"
)]
pub(crate) trait LockBefore<B> {}

impl<A, B: LockAfter<A>> LockBefore<B> for A {}

/// Proof that the code holding it holds locks up to level `L` and nothing
/// later.
///
/// A token is zero-sized and is not `Send`, since the locks it stands for
/// belong to this thread.
pub(crate) struct Ctx<'a, L> {
    /// Ties a token to the guard or token it was made from.
    _borrow: PhantomData<&'a mut ()>,
    _level: PhantomData<fn() -> L>,
    _not_send: PhantomData<*const ()>,
}

impl<L> Ctx<'_, L> {
    /// A token for level `L`. Only lock acquisition makes these.
    pub(super) fn new() -> Self {
        Self {
            _borrow: PhantomData,
            _level: PhantomData,
            _not_send: PhantomData,
        }
    }
}

impl Ctx<'static, levels::ThreadRoot> {
    /// The token for a thread that holds no locks.
    ///
    /// Only thread entry points make one: boot, the start of each kernel
    /// thread, and the kernel tests. A second root token on the same thread
    /// would let it take a lock it already holds. That deadlocks instead of
    /// corrupting memory, and the run-time checker reports it.
    pub(crate) fn root() -> Self {
        Self::new()
    }
}

/// Declares a chain of lock levels, each after the one before it.
macro_rules! lock_levels {
    ($(#[$meta:meta])* $root:ident $($rest:tt)*) => {
        $(#[$meta])*
        pub(crate) enum $root {}

        impl $crate::sync::level::Level for $root {
            const NAME: &'static str = stringify!($root);
            const RANK: u16 = 0;
        }

        lock_levels!(@after $root $($rest)*);
    };
    (@after $parent:ident => $(#[$meta:meta])* $level:ident $($rest:tt)*) => {
        $(#[$meta])*
        pub(crate) enum $level {}

        impl $crate::sync::level::Level for $level {
            const NAME: &'static str = stringify!($level);
            const RANK: u16 = <$parent as $crate::sync::level::Level>::RANK + 1;
        }

        impl $crate::sync::level::LockAfter<$parent> for $level {}
        impl<X: $crate::sync::level::LockBefore<$parent>> $crate::sync::level::LockAfter<X>
            for $level
        {
        }

        lock_levels!(@after $level $($rest)*);
    };
    (@after $parent:ident) => {};
}

/// The kernel's lock order, earliest first.
pub(crate) mod levels {
    lock_levels! {
        /// A thread that holds no locks.
        ThreadRoot
        // Sleeping locks.
        => /// A `Mutex` the kernel tests take first.
           KtestOuter
        => /// A `Mutex` the kernel tests take second.
           KtestInner
        // Sleeping is allowed with every level up to here.
        => /// Marks where sleeping stops being allowed.
           MaySleep
        // Interrupt handlers start here.
        => /// Where interrupt handlers start.
           IrqRoot
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
        => /// The kernel heap. `GlobalAlloc` takes it from anywhere, so it is
           /// last and takes no other lock.
           Heap
    }
}
