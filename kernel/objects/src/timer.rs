//! Timer object — deadline-based timer.
//!
//! A timer asserts `SIGNAL_0` when its deadline elapses. The actual firing
//! is driven by the kernel's timer subsystem calling [`Timer::trigger`].
//! Setting a new deadline or canceling clears the signal.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};

/// Timer state managed under a lock.
struct TimerState {
    /// Deadline in nanoseconds (monotonic clock). `None` = not armed.
    deadline: Option<u64>,
    /// Slack in nanoseconds for coalescing timer fires.
    slack: u64,
}

/// A timer — fires `SIGNAL_0` when the deadline elapses.
///
/// Timers are armed via [`set`](Timer::set) and can be canceled via
/// [`cancel`](Timer::cancel). The kernel's timer wheel calls
/// [`trigger`](Timer::trigger) when the deadline is reached.
pub struct Timer {
    /// Unique identifier.
    koid: Koid,
    /// Timer state (deadline + slack).
    state: SpinLock<TimerState>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Timer {
    /// Create a new unarmed timer.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            state: SpinLock::new(TimerState {
                deadline: None,
                slack: 0,
            }),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Arm the timer with a deadline and slack.
    ///
    /// If the timer was previously fired, `SIGNAL_0` is cleared.
    /// The kernel's timer subsystem should be notified to schedule the wakeup.
    pub fn set(&self, deadline: u64, slack: u64) {
        let mut state = self.state.lock();
        state.deadline = Some(deadline);
        state.slack = slack;
        drop(state);

        // Clear any previously-fired signal.
        self.signals
            .fetch_and(!Signals::SIGNAL_0.bits(), Ordering::Release);
    }

    /// Cancel the timer.
    ///
    /// Clears the deadline and any asserted `SIGNAL_0`.
    pub fn cancel(&self) {
        let mut state = self.state.lock();
        state.deadline = None;
        drop(state);

        self.signals
            .fetch_and(!Signals::SIGNAL_0.bits(), Ordering::Release);
    }

    /// Called by the kernel timer subsystem when the deadline elapses.
    ///
    /// Asserts `SIGNAL_0` and notifies observers.
    pub fn trigger(&self) {
        let mut state = self.state.lock();
        state.deadline = None;
        drop(state);

        signal_update(
            &self.signals,
            Signals::SIGNAL_0,
            Signals::empty(),
            &self.observers,
            self.koid,
        );
    }

    /// The current deadline, if armed.
    #[must_use]
    pub fn deadline(&self) -> Option<u64> {
        self.state.lock().deadline
    }
}

impl KernelObject for Timer {
    fn object_type(&self) -> ObjectType {
        ObjectType::Timer
    }

    fn koid(&self) -> Koid {
        self.koid
    }

    fn get_signals(&self) -> Signals {
        Signals::from_bits_truncate(self.signals.load(Ordering::Relaxed))
    }

    fn add_observer(&self, port: Arc<dyn PortDispatch>, key: u64, signals: Signals) {
        self.observers.add(port, key, signals);
    }

    fn remove_observer(&self, port: &Arc<dyn PortDispatch>) {
        self.observers.remove_by_port(port);
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use alloc::vec::Vec;

    use hadron_core::sync::SpinLock;

    use super::*;
    use crate::port_packet::PortPacket;

    struct MockPort {
        packets: SpinLock<Vec<PortPacket>>,
    }

    impl MockPort {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                packets: SpinLock::new(Vec::new()),
            })
        }

        fn take_packets(&self) -> Vec<PortPacket> {
            core::mem::take(&mut *self.packets.lock())
        }
    }

    impl PortDispatch for MockPort {
        fn queue_packet(&self, packet: PortPacket) {
            self.packets.lock().push(packet);
        }
    }

    #[test]
    fn timer_create_unarmed() {
        let timer = Timer::new();
        assert_eq!(timer.object_type(), ObjectType::Timer);
        assert!(timer.deadline().is_none());
        assert!(!timer.get_signals().contains(Signals::SIGNAL_0));
    }

    #[test]
    fn timer_set_and_trigger() {
        let timer = Timer::new();
        timer.set(1_000_000, 0);
        assert_eq!(timer.deadline(), Some(1_000_000));

        timer.trigger();
        assert!(timer.deadline().is_none());
        assert!(timer.get_signals().contains(Signals::SIGNAL_0));
    }

    #[test]
    fn timer_cancel_clears() {
        let timer = Timer::new();
        timer.set(1_000_000, 0);
        timer.trigger();
        assert!(timer.get_signals().contains(Signals::SIGNAL_0));

        timer.set(2_000_000, 0);
        // Re-arming clears SIGNAL_0.
        assert!(!timer.get_signals().contains(Signals::SIGNAL_0));

        timer.cancel();
        assert!(timer.deadline().is_none());
    }

    #[test]
    fn timer_trigger_notifies_observer() {
        let timer = Timer::new();
        let port = MockPort::new();

        timer.add_observer(port.clone(), 5, Signals::SIGNAL_0);
        timer.set(1_000_000, 0);
        timer.trigger();

        let packets = port.take_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].key, 5);
    }
}
