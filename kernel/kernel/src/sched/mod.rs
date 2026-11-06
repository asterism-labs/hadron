//! The scheduler: kernel threads, one run queue, and blocking.
//!
//! The boot context becomes the idle thread, which runs only when no other
//! thread is ready. Threads switch when they yield, park or exit. The
//! local APIC timer sets a reschedule flag on every tick, and the thread
//! switches when it next can: at the end of the interrupt, if it holds no
//! [`PreemptGuard`], or else when it drops the last one.
//!
//! Threads wait through `Waker`s. A thread's waker unparks it, so a wait
//! queue or a future wakes a thread the same way it would wake an async
//! task, and [`block_on`] runs a future on the current thread.
//!
//! # Switching
//!
//! [`schedule`] takes the run queue lock, picks the next thread, and
//! switches with the lock held. The thread switched to releases it: a
//! thread resuming in `schedule` drops its own guard from before its last
//! switch, and a new thread adopts the guard in [`finish_first_switch`].
//! The lock, the preempt count and the interrupt count all live on the CPU,
//! so either way the counts balance.
//!
//! The lock's [`IrqGuard`] restores the interrupt flag that the thread
//! which took it saw, so `schedule` swaps that flag per thread. A thread
//! switched out by an interrupt resumes with interrupts masked and turns
//! them back on as it returns from the interrupt.
//!
//! One CPU only. The run queue is global until SMP needs one per CPU.
//!
//! [`PreemptGuard`]: crate::percpu::PreemptGuard

mod thread;

use alloc::sync::Arc;
use core::pin::pin;
use core::ptr;
use core::sync::atomic::Ordering;
use core::task::{Context, Poll, Waker};

use thread::{EXITED, PARKED, READY, RUNNING};
pub(crate) use thread::{Thread, spawn};

use crate::arch::x86_64::{context, cpu};
use crate::percpu::{self, IrqGuard};
use crate::sync::{IrqSpinLock, levels};

/// Threads ready to run, oldest first, linked through [`Thread::next`].
struct ReadyList {
    head: Option<Arc<Thread>>,
    tail: *const Thread,
}

// SAFETY: `tail` points at the last thread in the list, which `head` owns.
// The list is used only under the run queue lock.
unsafe impl Send for ReadyList {}

impl ReadyList {
    const fn new() -> Self {
        Self {
            head: None,
            tail: ptr::null(),
        }
    }

    fn push(&mut self, thread: Arc<Thread>) {
        let raw = Arc::as_ptr(&thread);
        // SAFETY: `&mut self` means the run queue lock is held, which
        // guards every thread's `next`. A non-null `tail` is the last
        // thread in the list, kept alive by the list.
        match unsafe { self.tail.as_ref() } {
            None => self.head = Some(thread),
            Some(tail) => unsafe { *tail.next.get() = Some(thread) },
        }
        self.tail = raw;
    }

    fn pop(&mut self) -> Option<Arc<Thread>> {
        let head = self.head.take()?;
        // SAFETY: as in `push`.
        self.head = unsafe { (*head.next.get()).take() };
        if self.head.is_none() {
            self.tail = ptr::null();
        }
        Some(head)
    }

    fn is_empty(&self) -> bool {
        self.head.is_none()
    }
}

/// The run queue. Wakers take it, so it is IRQ-safe and late in the lock
/// order.
static RUN_QUEUE: IrqSpinLock<levels::RunQueue, ReadyList> = IrqSpinLock::new(ReadyList::new());

/// Turns the boot context into the idle thread. Called once, on the boot
/// stack, after the heap is up.
///
/// # Panics
///
/// Panics if called twice.
pub(crate) fn init() {
    let irq = IrqGuard::new();
    let cpu = percpu::current(&irq);
    assert!(cpu.idle.get().is_null(), "sched::init called twice");
    let idle = Arc::new(Thread::boot());
    cpu.current.set(Arc::into_raw(idle.clone()));
    cpu.idle.set(Arc::into_raw(idle));
}

/// Runs the idle loop on the boot thread for good. It halts the CPU
/// while no thread is ready, and an interrupt that wakes one ends the
/// halt.
///
/// # Panics
///
/// Panics if called with interrupts masked, since then no interrupt could
/// end the halt.
pub(crate) fn run_idle() -> ! {
    loop {
        let enabled = cpu::save_and_disable_interrupts();
        assert!(enabled, "the idle thread runs with interrupts enabled");
        if RUN_QUEUE.lock().is_empty() {
            // SAFETY: boot enabled interrupts, so every interrupt that can
            // arrive has a handler, and no `IrqGuard` is alive here.
            unsafe { cpu::enable_interrupts_and_halt() };
        } else {
            cpu::restore_interrupts(enabled);
        }
        yield_now();
    }
}

/// Panics if the current thread may not sleep: it holds a spin lock, or
/// has preemption or interrupts masked. Every function that may sleep
/// calls this first, so the mistake shows on every call, not only when
/// the function happens to sleep.
#[track_caller]
pub(crate) fn might_sleep() {
    let depth = percpu::preempt_depth();
    assert!(
        depth == 0,
        "may sleep with preemption off (depth {depth}): a spin lock is held, or \
         interrupts or preemption are masked"
    );
}

/// The running thread.
///
/// # Panics
///
/// Panics before [`init`].
pub(crate) fn current() -> Arc<Thread> {
    let irq = IrqGuard::new();
    let raw = percpu::current(&irq).current.get();
    assert!(!raw.is_null(), "sched::init ran");
    // SAFETY: `current` holds a reference from `Arc::into_raw`, so the
    // count is at least 1 while the guard keeps this thread running.
    unsafe {
        Arc::increment_strong_count(raw);
        Arc::from_raw(raw)
    }
}

/// Moves the current thread to the back of the run queue and runs the
/// next ready thread, if any.
///
/// # Panics
///
/// Panics if the thread may not sleep (see [`might_sleep`]).
#[track_caller]
pub(crate) fn yield_now() {
    might_sleep();
    schedule(Switch::Yield);
}

/// Sleeps until [`unpark`] is called for this thread. Returns at once if
/// an unpark came since the last park. It can also return early, so
/// callers check their condition in a loop.
///
/// # Panics
///
/// Panics if the thread may not sleep (see [`might_sleep`]).
#[track_caller]
pub(crate) fn park() {
    might_sleep();
    let thread = current();
    if thread.notified.swap(false, Ordering::Acquire) {
        return;
    }
    drop(thread);
    schedule(Switch::Park);
}

/// Wakes `thread` if it is parked, or makes its next park return at once.
pub(crate) fn unpark(thread: &Arc<Thread>) {
    // Paired with `schedule` storing `PARKED` and then reading `notified`:
    // with both sequentially consistent, either the parking thread sees
    // the flag or this sees `PARKED`.
    thread.notified.store(true, Ordering::SeqCst);
    if thread.state.load(Ordering::SeqCst) == PARKED {
        let mut queue = RUN_QUEUE.lock();
        let woke = thread
            .state
            .compare_exchange(PARKED, READY, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if woke {
            queue.push(thread.clone());
        }
    }
}

/// Runs `future` to completion on the current thread, parking whenever it
/// is pending. The future's waker unparks this thread.
///
/// # Panics
///
/// Panics if the thread may not sleep (see [`might_sleep`]).
#[track_caller]
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    might_sleep();
    let waker = Waker::from(current());
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        park();
    }
}

/// Puts a new or woken thread at the back of the run queue.
fn enqueue(thread: Arc<Thread>) {
    RUN_QUEUE.lock().push(thread);
}

/// Called at the end of an interrupt. Switches threads if a reschedule is
/// pending and the interrupted thread holds no [`PreemptGuard`]. The
/// switch returns when the thread runs again, and the interrupt returns to
/// it then.
pub(crate) fn preempt_on_interrupt_exit() {
    if percpu::preempt_depth() == 0 && percpu::need_resched() {
        preempt();
    }
}

/// Called by `PreemptGuard` when the last guard drops with a reschedule
/// pending. It makes no guard of its own: dropping one would find the
/// reschedule still pending and call this again.
pub(crate) fn preempt() {
    if percpu::has_current_thread() {
        schedule(Switch::Yield);
    }
}

/// Why the current thread gives up the CPU.
#[derive(Clone, Copy)]
enum Switch {
    /// It stays ready and goes to the back of the run queue.
    Yield,
    /// It sleeps until unparked.
    Park,
    /// It never runs again.
    Exit,
}

/// Switches to the next ready thread, or the idle thread if none is ready.
/// Returns when this thread runs again, or at once if it should keep
/// running.
fn schedule(how: Switch) {
    // Switching with a spin lock held or interrupts masked would carry
    // the lock or the masked state into the next thread.
    assert_eq!(
        percpu::preempt_depth(),
        0,
        "a thread gave up the CPU with preemption disabled"
    );
    let mut queue = RUN_QUEUE.lock();
    let (current_raw, idle_raw) = {
        let cpu = percpu::current(queue.mask());
        cpu.clear_need_resched();
        (cpu.current.get(), cpu.idle.get())
    };
    // SAFETY: `current` holds a reference to the running thread.
    let current = unsafe { &*current_raw };

    match how {
        Switch::Yield if current_raw != idle_raw => {
            current.state.store(READY, Ordering::Relaxed);
            // SAFETY: as above. The queue gets a reference of its own.
            queue.push(unsafe {
                Arc::increment_strong_count(current_raw);
                Arc::from_raw(current_raw)
            });
        }
        Switch::Yield => {}
        Switch::Park => {
            assert!(current_raw != idle_raw, "the idle thread parked");
            current.state.store(PARKED, Ordering::SeqCst);
            if current.notified.swap(false, Ordering::SeqCst) {
                current.state.store(RUNNING, Ordering::Relaxed);
                return;
            }
        }
        Switch::Exit => current.state.store(EXITED, Ordering::Relaxed),
    }

    let next = match queue.pop() {
        Some(next) => next,
        None if current_raw == idle_raw => return,
        // SAFETY: `idle` holds a reference to the idle thread, which never
        // exits. `current` gets a reference of its own.
        None => unsafe {
            Arc::increment_strong_count(idle_raw);
            Arc::from_raw(idle_raw)
        },
    };
    if ptr::eq(Arc::as_ptr(&next), current_raw) {
        // It yielded with no other thread ready.
        current.state.store(RUNNING, Ordering::Relaxed);
        return;
    }
    next.state.store(RUNNING, Ordering::Relaxed);
    let next_raw = Arc::into_raw(next);
    let cpu = percpu::current(queue.mask());
    cpu.current.set(next_raw);
    // The outgoing thread's reference moves to `previous`, and the thread
    // switched to drops it.
    cpu.previous.set(current_raw);
    // SAFETY: the run queue lock is held, which guards both flags.
    unsafe {
        let next_irq_enabled = *(*next_raw).irq_enabled.get();
        *current.irq_enabled.get() = cpu.replace_irq_enabled(next_irq_enabled);
    }

    // SAFETY: the run queue lock is held, which guards both `rsp`s.
    // `next` was switched out by `switch` or built by `initial_stack`,
    // and its stack is mapped and unused. `previous` keeps the outgoing
    // thread, and the stack this runs on, alive until the switch is done.
    unsafe {
        let new_rsp = *(*next_raw).rsp.get();
        context::switch(current.rsp.get(), new_rsp);
    }
    finish_switch(queue);
}

/// The second half of a switch, run by the thread switched to: releases
/// the run queue lock, then drops the outgoing thread's reference.
fn finish_switch(queue: crate::sync::IrqSpinLockGuard<'_, levels::RunQueue, ReadyList>) {
    let previous = percpu::current(queue.mask()).previous.replace(ptr::null());
    drop(queue);
    if !previous.is_null() {
        // SAFETY: `schedule` moved this reference from `current`. The
        // switch is done, so its stack is no longer in use.
        drop(unsafe { Arc::from_raw(previous) });
    }
}

/// The second half of a new thread's first switch. The thread that
/// switched here holds the run queue lock through a guard it will never
/// drop, so this thread adopts it.
fn finish_first_switch() {
    // SAFETY: a new thread runs only through `schedule`, which holds the
    // lock with one forgotten-here guard: the old thread drops its guard
    // only after it is switched back to, and by then another switch has
    // taken the lock again.
    let queue = unsafe { RUN_QUEUE.assume_locked() };
    finish_switch(queue);
}

ktest::ktest! {
    fn spawned_thread_runs_and_joins() {
        use core::sync::atomic::AtomicBool;

        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let handle = spawn("ktest", move || flag.store(true, Ordering::SeqCst));
        assert!(handle.thread().id() > 0);
        assert_eq!(handle.thread().name(), "ktest");
        handle.join();
        assert!(ran.load(Ordering::SeqCst));
    }
}

ktest::ktest! {
    fn yield_takes_turns_in_arrival_order() {
        use alloc::vec::Vec;
        use crate::sync::Mutex;

        static ORDER: Mutex<levels::KtestOuter, Vec<u8>> = Mutex::new(Vec::new());
        let turns = |id: u8| {
            move || {
                for _ in 0..3 {
                    ORDER.lock().push(id);
                    yield_now();
                }
            }
        };
        let first = spawn("first", turns(1));
        let second = spawn("second", turns(2));
        first.join();
        second.join();
        assert_eq!(*ORDER.lock(), [1, 2, 1, 2, 1, 2]);
    }
}

ktest::ktest! {
    fn mutex_waiter_sleeps_until_release() {
        use crate::sync::Mutex;

        static VALUE: Mutex<levels::KtestOuter, u32> = Mutex::new(0);
        let mut guard = VALUE.lock();
        let waiter = spawn("waiter", || *VALUE.lock() += 1);
        // The waiter runs, finds the lock taken and parks.
        yield_now();
        assert_eq!(waiter.thread().state.load(Ordering::SeqCst), PARKED);
        *guard = 10;
        drop(guard);
        waiter.join();
        assert_eq!(*VALUE.lock(), 11);
    }
}

ktest::ktest! {
    fn nested_mutexes_follow_the_level_order() {
        use crate::sync::Mutex;

        static OUTER: Mutex<levels::KtestOuter, u32> = Mutex::new(1);
        static INNER: Mutex<levels::KtestInner, u32> = Mutex::new(2);
        let outer = OUTER.lock();
        let inner = INNER.lock();
        assert_eq!(*outer + *inner, 3);
    }
}

ktest::ktest! {
    fn wait_queue_wakes_a_blocked_thread() {
        use core::sync::atomic::AtomicBool;
        use crate::sync::WaitQueue;

        static QUEUE: WaitQueue = WaitQueue::new();
        static READY_FLAG: AtomicBool = AtomicBool::new(false);
        let waiter = spawn("waiter", || {
            QUEUE.block_until(|| READY_FLAG.load(Ordering::SeqCst));
        });
        yield_now();
        assert_eq!(waiter.thread().state.load(Ordering::SeqCst), PARKED);
        READY_FLAG.store(true, Ordering::SeqCst);
        QUEUE.wake_all();
        waiter.join();
    }
}

ktest::ktest! {
    fn dropping_the_last_preempt_guard_switches() {
        use core::sync::atomic::AtomicBool;
        use crate::percpu::PreemptGuard;

        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        let other = spawn("other", move || flag.store(true, Ordering::SeqCst));
        let preempt = PreemptGuard::new();
        {
            let irq = IrqGuard::new();
            percpu::current(&irq).set_need_resched();
        }
        assert!(!ran.load(Ordering::SeqCst), "a guard was held");
        drop(preempt);
        assert!(ran.load(Ordering::SeqCst), "the switch ran the other thread");
        other.join();
    }
}

ktest::ktest! {
    fn timer_preempts_a_spinning_thread() {
        use core::sync::atomic::AtomicBool;
        use crate::arch::x86_64::{cpu, lapic};

        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        // This thread starts from the switch the timer interrupt makes. It
        // must still run with interrupts enabled.
        let other = spawn("other", move || {
            assert!(cpu::interrupts_enabled(), "interrupts are enabled");
            flag.store(true, Ordering::SeqCst);
        });
        let start = lapic::ticks();
        while !ran.load(Ordering::SeqCst) && lapic::ticks() < start + 10 {
            core::hint::spin_loop();
        }
        assert!(ran.load(Ordering::SeqCst), "the timer switched threads");
        assert!(cpu::interrupts_enabled());
        other.join();
    }
}

ktest::ktest! {
    fn exited_threads_give_back_their_stacks() {
        spawn("first", || {}).join();
        let mapped = thread::mapped_stacks();
        for _ in 0..32 {
            spawn("again", || {}).join();
        }
        assert_eq!(thread::mapped_stacks(), mapped);
    }
}

ktest::ktest! {
    #[ignore]
    fn might_sleep_reports_sleeping_under_a_spin_lock() {
        use crate::sync::{IrqSpinLock, levels::KtestSpin};
        // Shows the report. Run it with
        // `QUON_KTEST_IGNORED=true quon run -p test`.
        static LOCK: IrqSpinLock<KtestSpin, ()> = IrqSpinLock::new(());
        let _held = LOCK.lock();
        yield_now();
    }
}
