//! Kernel time: a clock source, instants, sleeping and timeouts.
//!
//! [`now`] reads the [`ClockSource`] that boot installs and converts its
//! count to nanoseconds since the count started. A deadline adds one
//! count of the source on top of the duration, since the current count is
//! already partly over, so a sleep never ends early.
//!
//! Sleeping threads wait in one timer queue, ordered by deadline. The
//! local APIC timer interrupt wakes every waiter whose deadline has
//! passed, so wakeups land on its ticks whatever the clock source. A
//! [`Sleep`] dropped before its deadline takes its waker out of the queue.

use alloc::collections::BTreeMap;
use core::ops::Add;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;

use crate::sched;
use crate::sync::{IrqSpinLock, WriteOnce, levels};

const NANOS_PER_SEC: u64 = 1_000_000_000;

/// A counter that time is read from, such as a timer tick count, the TSC
/// or the HPET main counter.
pub(crate) trait ClockSource: Sync {
    /// A short name for boot messages.
    fn name(&self) -> &'static str;

    /// The current count. It rises monotonically and never wraps, so a
    /// source with a narrower counter must extend it.
    fn read(&self) -> u64;

    /// Counts per second. Fixed for the life of the source.
    fn frequency(&self) -> u64;
}

/// The installed source, with its frequency and resolution read once.
struct Clock {
    source: &'static dyn ClockSource,
    frequency: u64,
    /// One count in nanoseconds, rounded up.
    resolution: u64,
}

static CLOCK: WriteOnce<Clock> = WriteOnce::new();

/// Makes `source` the clock that [`now`] reads. Boot calls this once.
///
/// # Panics
///
/// Panics if a source is already installed or `source` counts at 0 Hz.
pub(crate) fn install(source: &'static dyn ClockSource) {
    let frequency = source.frequency();
    assert!(
        frequency > 0,
        "clock source {} counts at 0 Hz",
        source.name()
    );
    CLOCK.init(Clock {
        source,
        frequency,
        resolution: resolution(frequency),
    });
}

fn clock() -> &'static Clock {
    CLOCK.get().expect("time::install ran")
}

/// The name of the installed clock source.
///
/// # Panics
///
/// Panics before [`install`].
pub(crate) fn source_name() -> &'static str {
    clock().source.name()
}

/// `count` at `frequency` in nanoseconds, rounded down. Saturates at
/// `u64::MAX`.
fn to_nanos(count: u64, frequency: u64) -> u64 {
    let nanos = u128::from(count) * u128::from(NANOS_PER_SEC) / u128::from(frequency);
    u64::try_from(nanos).unwrap_or(u64::MAX)
}

/// One count at `frequency` in nanoseconds, rounded up.
fn resolution(frequency: u64) -> u64 {
    NANOS_PER_SEC.div_ceil(frequency)
}

/// A point in time, in nanoseconds since the clock source started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Instant {
    nanos: u64,
}

impl Instant {
    /// The time from `earlier` to this instant, or zero if `earlier` is
    /// later.
    pub(crate) fn duration_since(self, earlier: Self) -> Duration {
        Duration::from_nanos(self.nanos.saturating_sub(earlier.nanos))
    }
}

impl Add<Duration> for Instant {
    type Output = Self;

    /// The instant `duration` later. Saturates at the last representable
    /// nanosecond, about 584 years in.
    fn add(self, duration: Duration) -> Self {
        let nanos = u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX);
        Self {
            nanos: self.nanos.saturating_add(nanos),
        }
    }
}

/// The current instant.
///
/// # Panics
///
/// Panics before [`install`].
pub(crate) fn now() -> Instant {
    let clock = clock();
    Instant {
        nanos: to_nanos(clock.source.read(), clock.frequency),
    }
}

/// The earliest deadline that is at least `duration` from now.
///
/// [`now`] rounds down to the last whole count, so this adds one count
/// of the source on top of `now() + duration`.
///
/// # Panics
///
/// Panics before [`install`].
pub(crate) fn deadline_after(duration: Duration) -> Instant {
    now() + duration + Duration::from_nanos(clock().resolution)
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
    let now = now();
    loop {
        // One waker at a time, woken with the lock released, since waking
        // takes the run queue lock.
        let waker = {
            let mut timers = TIMERS.lock();
            match timers.waiting.first_entry() {
                Some(entry) if entry.key().0 <= now.nanos => entry.remove(),
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
        (self.deadline.nanos, id)
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let mut timers = TIMERS.lock();
        // Checked under the lock: a timer tick after this check runs
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
    fn counts_convert_to_nanoseconds() {
        // 100 Hz ticks, and a 3 GHz TSC.
        assert_eq!(to_nanos(3, 100), 30_000_000);
        assert_eq!(resolution(100), 10_000_000);
        assert_eq!(to_nanos(3_000_000_000, 3_000_000_000), NANOS_PER_SEC);
        assert_eq!(to_nanos(1, 3_000_000_000), 0);
        assert_eq!(resolution(3_000_000_000), 1);
        // A year at 3 GHz overflows `count * 1e9` in 64 bits.
        let year = 365 * 24 * 3600;
        assert_eq!(to_nanos(year * 3_000_000_000, 3_000_000_000), year * NANOS_PER_SEC);
        assert_eq!(to_nanos(u64::MAX, 1), u64::MAX);
    }
}

ktest::ktest! {
    fn deadlines_add_one_count_of_the_source() {
        let before = now();
        let deadline = deadline_after(Duration::from_millis(5));
        let added = deadline.duration_since(before);
        let least = Duration::from_millis(5) + Duration::from_nanos(clock().resolution);
        assert!(added >= least, "{added:?} added, expected at least {least:?}");
        assert_eq!(Instant { nanos: u64::MAX - 1 } + Duration::MAX, Instant { nanos: u64::MAX });
    }
}

ktest::ktest! {
    fn sleep_lasts_at_least_its_duration() {
        let start = now();
        sleep(Duration::from_millis(30));
        let slept = now().duration_since(start);
        // Wakeups land on 10 ms ticks, and under emulation a tick can come
        // late, so the upper bound leaves room.
        assert!(
            (Duration::from_millis(30)..=Duration::from_millis(80)).contains(&slept),
            "slept {slept:?} for 30 ms"
        );
    }
}

ktest::ktest! {
    fn sleep_until_a_past_deadline_returns_at_once() {
        let start = now();
        sleep_until(start);
        assert!(now().duration_since(start) <= Duration::from_millis(10));
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
