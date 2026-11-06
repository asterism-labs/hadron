//! Kernel time: instants, sleeping and timeouts.
//!
//! Time counts local APIC timer ticks, [`HZ`] per second. A deadline is
//! the first tick at or after it, so a duration rounds up to whole ticks
//! and a sleep never ends early.
//!
//! Sleeping threads wait in one timer queue, ordered by deadline. The
//! timer interrupt wakes every waiter whose deadline has passed. A
//! [`Sleep`] dropped before its deadline takes its waker out of the queue.

use alloc::collections::BTreeMap;
use core::ops::Add;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;

use crate::arch::x86_64::lapic::{self, HZ};
use crate::sched;
use crate::sync::{IrqSpinLock, levels};

/// A point in time, counted in timer ticks since the timer started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Instant {
    ticks: u64,
}

impl Instant {
    /// Timer ticks since the timer started.
    pub(crate) fn ticks(self) -> u64 {
        self.ticks
    }
}

impl Add<Duration> for Instant {
    type Output = Self;

    /// The instant `duration` later, rounded up to a whole tick. Saturates
    /// at the last representable tick.
    fn add(self, duration: Duration) -> Self {
        Self {
            ticks: self.ticks.saturating_add(ticks_in(duration)),
        }
    }
}

/// `duration` in timer ticks, rounded up.
fn ticks_in(duration: Duration) -> u64 {
    let ticks = (duration.as_nanos() * u128::from(HZ)).div_ceil(1_000_000_000);
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The current instant.
pub(crate) fn now() -> Instant {
    Instant {
        ticks: lapic::ticks(),
    }
}

/// The earliest deadline that is at least `duration` from now.
///
/// The current tick is already partly over, so this adds one tick on top
/// of `now() + duration`.
pub(crate) fn deadline_after(duration: Duration) -> Instant {
    Instant {
        ticks: (now() + duration).ticks.saturating_add(1),
    }
}

/// Blocks the current thread for at least `duration`.
///
/// # Panics
///
/// Panics if the thread may not sleep (see [`sched::might_sleep`]).
#[track_caller]
pub(crate) fn sleep(duration: Duration) {
    sleep_until(deadline_after(duration));
}

/// Blocks the current thread until `deadline`. Returns at once if it has
/// passed.
///
/// # Panics
///
/// Panics if the thread may not sleep (see [`sched::might_sleep`]).
#[track_caller]
pub(crate) fn sleep_until(deadline: Instant) {
    sched::block_on(Sleep::until(deadline));
}

/// Wakers waiting for a deadline, keyed by deadline and then by an ID
/// that keeps equal deadlines apart.
struct Timers {
    next_id: u64,
    waiting: BTreeMap<(u64, u64), Waker>,
}

/// The timer queue. The timer interrupt takes it, so it is IRQ-safe.
static TIMERS: IrqSpinLock<levels::Timers, Timers> = IrqSpinLock::new(Timers {
    next_id: 0,
    waiting: BTreeMap::new(),
});

/// Wakes every waiter whose deadline has passed. The timer interrupt calls
/// this after it counts the tick.
pub(crate) fn expire() {
    let now = now().ticks;
    loop {
        // One waker at a time, woken with the lock released, since waking
        // takes the run queue lock.
        let waker = {
            let mut timers = TIMERS.lock();
            match timers.waiting.first_entry() {
                Some(entry) if entry.key().0 <= now => entry.remove(),
                _ => break,
            }
        };
        waker.wake();
    }
}

/// How many wakers wait in the timer queue.
fn waiting() -> usize {
    TIMERS.lock().waiting.len()
}

/// A future that completes at a deadline.
#[derive(Debug)]
pub(crate) struct Sleep {
    deadline: Instant,
    /// The ID of this future's entry in the timer queue, if it has one.
    id: Option<u64>,
}

impl Sleep {
    /// A future that completes at `deadline`.
    pub(crate) fn until(deadline: Instant) -> Self {
        Self { deadline, id: None }
    }

    fn key(&self, id: u64) -> (u64, u64) {
        (self.deadline.ticks, id)
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let mut timers = TIMERS.lock();
        // Checked under the lock: a tick counted after this load runs
        // `expire` only once the lock is free, and finds the entry.
        if now() >= this.deadline {
            let old = this
                .id
                .take()
                .and_then(|id| timers.waiting.remove(&this.key(id)));
            drop(timers);
            // Dropped with the lock released, as it may be the last
            // reference to a thread.
            drop(old);
            return Poll::Ready(());
        }
        let old = match this.id.and_then(|id| timers.waiting.get_mut(&this.key(id))) {
            Some(waker) if waker.will_wake(cx.waker()) => None,
            Some(waker) => Some(core::mem::replace(waker, cx.waker().clone())),
            None => {
                let id = timers.next_id;
                timers.next_id += 1;
                timers.waiting.insert(this.key(id), cx.waker().clone());
                this.id = Some(id);
                None
            }
        };
        drop(timers);
        drop(old);
        Poll::Pending
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let old = TIMERS.lock().waiting.remove(&self.key(id));
            drop(old);
        }
    }
}

/// The error a [`timeout`] future returns when its deadline passes first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TimedOut;

/// Runs `future` until it completes or `deadline` passes, whichever comes
/// first. If both are ready at the same poll, the future's output wins.
pub(crate) fn timeout<F: Future>(deadline: Instant, future: F) -> Timeout<F> {
    Timeout {
        future,
        sleep: Sleep::until(deadline),
    }
}

/// The future [`timeout`] returns.
#[derive(Debug)]
pub(crate) struct Timeout<F> {
    /// Pinned structurally: never moved out of a pinned `Timeout`.
    future: F,
    sleep: Sleep,
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, TimedOut>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: nothing moves out of `this`. `future` is only used
        // through the pin made here, and `Timeout` has no `Drop` that
        // could move it.
        let this = unsafe { self.get_unchecked_mut() };
        // SAFETY: `this` is pinned, so `future` is too.
        let future = unsafe { Pin::new_unchecked(&mut this.future) };
        if let Poll::Ready(output) = future.poll(cx) {
            return Poll::Ready(Ok(output));
        }
        Pin::new(&mut this.sleep).poll(cx).map(|()| Err(TimedOut))
    }
}

ktest::ktest! {
    fn durations_round_up_to_whole_ticks() {
        let tick = Duration::from_nanos(1_000_000_000 / HZ);
        assert_eq!(ticks_in(Duration::ZERO), 0);
        assert_eq!(ticks_in(Duration::from_nanos(1)), 1);
        assert_eq!(ticks_in(tick), 1);
        assert_eq!(ticks_in(tick + Duration::from_nanos(1)), 2);
        assert_eq!(ticks_in(Duration::MAX), u64::MAX);
    }
}

ktest::ktest! {
    fn sleep_lasts_at_least_its_duration() {
        let start = now();
        sleep(Duration::from_millis(30));
        let slept = now().ticks() - start.ticks();
        // 30 ms is 3 ticks. Under emulation a tick can come late, so the
        // upper bound leaves room.
        assert!((3..=8).contains(&slept), "slept {slept} ticks for 30 ms");
    }
}

ktest::ktest! {
    fn sleep_until_a_past_deadline_returns_at_once() {
        let start = now();
        sleep_until(start);
        assert!(now().ticks() - start.ticks() <= 1);
    }
}

ktest::ktest! {
    fn sleepers_wake_in_deadline_order() {
        use alloc::vec::Vec;
        use crate::sync::Mutex;

        static ORDER: Mutex<levels::KtestOuter, Vec<u8>> = Mutex::new(Vec::new());
        let sleeper = |id: u8, ms: u64| {
            move || {
                sleep(Duration::from_millis(ms));
                ORDER.lock().push(id);
            }
        };
        let late = sched::spawn("late", sleeper(1, 50));
        let early = sched::spawn("early", sleeper(2, 10));
        late.join();
        early.join();
        assert_eq!(*ORDER.lock(), [2, 1]);
    }
}

ktest::ktest! {
    fn dropping_a_sleep_leaves_the_timer_queue() {
        let before = waiting();
        let mut sleep = Sleep::until(deadline_after(Duration::from_secs(60)));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(Pin::new(&mut sleep).poll(&mut cx).is_pending());
        assert_eq!(waiting(), before + 1);
        drop(sleep);
        assert_eq!(waiting(), before);
    }
}

ktest::ktest! {
    fn timeout_prefers_a_ready_future() {
        let mut both = core::pin::pin!(timeout(now(), core::future::ready(7)));
        let mut cx = Context::from_waker(Waker::noop());
        assert_eq!(both.as_mut().poll(&mut cx), Poll::Ready(Ok(7)));
    }
}
