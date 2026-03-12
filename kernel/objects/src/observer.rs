//! Observer infrastructure for async signal notification.
//!
//! The observer system connects kernel objects to [`Port`](super::port::Port)s.
//! When an object's signals change, registered observers queue
//! [`PortPacket`](super::port_packet::PortPacket)s to their target ports.
//!
//! # Architecture
//!
//! - [`PortDispatch`] — trait implemented by Port for receiving packets
//! - [`Observer`] — a single registration (port + key + signal mask)
//! - [`ObserverList`] — per-object list of observers with thread-safe mutation

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::Waker;

use hadron_core::sync::SpinLock;

use crate::object::{Koid, Signals};
use crate::port_packet::{PacketType, PortPacket};

/// Trait for objects that can receive [`PortPacket`]s (i.e., Port).
///
/// This is separate from [`KernelObject`](crate::object::KernelObject) to avoid
/// a circular dependency: observers need to queue packets, but the port is
/// itself a kernel object.
pub trait PortDispatch: Send + Sync + 'static {
    /// Queue a packet for delivery. The port wakes any blocked waiters.
    fn queue_packet(&self, packet: PortPacket);
}

/// A single observer registration on a kernel object.
struct Observer {
    /// The port that will receive notifications.
    port: Arc<dyn PortDispatch>,
    /// Caller-supplied key included in delivered packets.
    key: u64,
    /// Which signals this observer is interested in.
    mask: Signals,
}

/// Thread-safe list of observers attached to a kernel object.
///
/// Observers are added by `object_wait_async` and removed when the port is
/// closed or the observer fires (one-shot). The notify path snapshots the
/// list under lock, then delivers packets outside the lock to avoid lock
/// ordering issues.
pub struct ObserverList {
    inner: SpinLock<Vec<Observer>>,
}

impl ObserverList {
    /// Create an empty observer list.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: SpinLock::new(Vec::new()),
        }
    }

    /// Register an observer for the given signal mask.
    pub fn add(&self, port: Arc<dyn PortDispatch>, key: u64, mask: Signals) {
        self.inner.lock().push(Observer { port, key, mask });
    }

    /// Register an observer and immediately check if signals already match.
    ///
    /// If `current_signals` already intersects `mask`, the observer fires
    /// immediately (one-shot) and is never stored. This avoids the race
    /// where signals are asserted between the `add_observer` call and
    /// a subsequent check.
    pub fn add_and_check(
        &self,
        port: Arc<dyn PortDispatch>,
        key: u64,
        mask: Signals,
        current_signals: Signals,
        koid: Koid,
    ) {
        if mask.intersects(current_signals) {
            // Already satisfied — fire immediately without storing.
            port.queue_packet(PortPacket {
                key,
                signals: current_signals,
                koid,
                packet_type: PacketType::SignalOne,
            });
        } else {
            self.inner.lock().push(Observer { port, key, mask });
        }
    }

    /// Remove all observers targeting the given port (compared by `Arc` pointer).
    pub fn remove_by_port(&self, port: &Arc<dyn PortDispatch>) {
        let target = Arc::as_ptr(port);
        self.inner
            .lock()
            .retain(|o| !core::ptr::eq(Arc::as_ptr(&o.port), target));
    }

    /// Notify observers of newly-asserted signals.
    ///
    /// Takes a snapshot of matching observers under the lock, then delivers
    /// packets outside the lock. One-shot observers are removed after firing.
    pub fn notify(&self, signals: Signals, koid: Koid) {
        // Snapshot matching observers under lock, removing one-shot entries.
        let to_deliver: Vec<(Arc<dyn PortDispatch>, u64, Signals)> = {
            let mut list = self.inner.lock();
            let mut deliver = Vec::new();

            // Drain-filter: remove observers that match (one-shot semantics).
            let mut i = 0;
            while i < list.len() {
                if list[i].mask.intersects(signals) {
                    let obs = list.swap_remove(i);
                    deliver.push((obs.port, obs.key, signals));
                    // Don't increment i — swap_remove moved the last element here.
                } else {
                    i += 1;
                }
            }

            deliver
        };

        // Deliver outside the lock to avoid lock ordering issues.
        for (port, key, sigs) in to_deliver {
            port.queue_packet(PortPacket {
                key,
                signals: sigs,
                koid,
                packet_type: PacketType::SignalOne,
            });
        }
    }
}

/// Atomically update an object's signal state and notify observers.
///
/// Sets bits in `set_mask` and clears bits in `clear_mask`, then notifies
/// observers of any newly-asserted signals.
///
/// # Arguments
///
/// * `signals` — the object's `AtomicU32` signal storage
/// * `set_mask` — signals to assert
/// * `clear_mask` — signals to deassert
/// * `observers` — the object's observer list
/// * `koid` — the object's koid (included in delivered packets)
pub fn signal_update(
    signals: &AtomicU32,
    set_mask: Signals,
    clear_mask: Signals,
    observers: &ObserverList,
    koid: Koid,
) {
    let old = signals.fetch_update(Ordering::Release, Ordering::Relaxed, |current| {
        Some((current | set_mask.bits()) & !clear_mask.bits())
    });

    // Determine newly-asserted signals.
    if let Ok(old_bits) = old {
        let old_signals = Signals::from_bits_truncate(old_bits);
        let newly_set = set_mask.difference(old_signals);
        if !newly_set.is_empty() {
            observers.notify(newly_set, koid);
        }
    }
}

/// A one-shot waker dispatch for async futures waiting on object signals.
///
/// Implements [`PortDispatch`] so it can be registered as an observer on any
/// kernel object. When a matching signal fires, `queue_packet` takes the
/// stored waker and calls `wake()`, unblocking the async future.
pub struct WakerDispatch {
    waker: SpinLock<Option<Waker>>,
}

impl WakerDispatch {
    /// Create a new `WakerDispatch` storing the given waker.
    #[must_use]
    pub fn new(waker: Waker) -> Self {
        Self {
            waker: SpinLock::new(Some(waker)),
        }
    }
}

impl PortDispatch for WakerDispatch {
    fn queue_packet(&self, _packet: PortPacket) {
        if let Some(waker) = self.waker.lock().take() {
            waker.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use alloc::vec::Vec;

    use hadron_core::sync::SpinLock;

    use super::*;

    /// Mock port that collects delivered packets.
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
    fn observer_fires_on_matching_signal() {
        let list = ObserverList::new();
        let port = MockPort::new();
        let koid = Koid::alloc();

        list.add(port.clone(), 42, Signals::READABLE);
        list.notify(Signals::READABLE, koid);

        let packets = port.take_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].key, 42);
        assert_eq!(packets[0].koid, koid);
        assert!(packets[0].signals.contains(Signals::READABLE));
    }

    #[test]
    fn observer_does_not_fire_on_non_matching_signal() {
        let list = ObserverList::new();
        let port = MockPort::new();

        list.add(port.clone(), 1, Signals::WRITABLE);
        list.notify(Signals::READABLE, Koid::alloc());

        assert!(port.take_packets().is_empty());
    }

    #[test]
    fn observer_is_one_shot() {
        let list = ObserverList::new();
        let port = MockPort::new();
        let koid = Koid::alloc();

        list.add(port.clone(), 1, Signals::READABLE);
        list.notify(Signals::READABLE, koid);
        list.notify(Signals::READABLE, koid);

        // Only one packet — observer was removed after first fire.
        assert_eq!(port.take_packets().len(), 1);
    }

    #[test]
    fn remove_by_port_clears_observers() {
        let list = ObserverList::new();
        let port = MockPort::new();

        list.add(port.clone(), 1, Signals::READABLE);
        list.remove_by_port(&(port.clone() as Arc<dyn PortDispatch>));
        list.notify(Signals::READABLE, Koid::alloc());

        assert!(port.take_packets().is_empty());
    }

    #[test]
    fn signal_update_notifies_newly_set() {
        let signals = AtomicU32::new(0);
        let observers = ObserverList::new();
        let port = MockPort::new();
        let koid = Koid::alloc();

        observers.add(port.clone(), 10, Signals::READABLE);
        signal_update(
            &signals,
            Signals::READABLE,
            Signals::empty(),
            &observers,
            koid,
        );

        assert_eq!(port.take_packets().len(), 1);
    }

    #[test]
    fn signal_update_does_not_re_notify() {
        let signals = AtomicU32::new(Signals::READABLE.bits());
        let observers = ObserverList::new();
        let port = MockPort::new();

        observers.add(port.clone(), 10, Signals::READABLE);
        // READABLE is already set — no newly-asserted signals.
        signal_update(
            &signals,
            Signals::READABLE,
            Signals::empty(),
            &observers,
            Koid::alloc(),
        );

        assert!(port.take_packets().is_empty());
    }

    #[test]
    fn waker_dispatch_wakes_on_packet() {
        let (waker, count) = crate::test_util::counting_waker();
        let dispatch = Arc::new(WakerDispatch::new(waker));
        let packet = PortPacket {
            key: 0,
            signals: Signals::READABLE,
            koid: Koid::alloc(),
            packet_type: PacketType::SignalOne,
        };
        dispatch.queue_packet(packet);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn waker_dispatch_is_one_shot() {
        let (waker, count) = crate::test_util::counting_waker();
        let dispatch = Arc::new(WakerDispatch::new(waker));
        let packet = PortPacket {
            key: 0,
            signals: Signals::READABLE,
            koid: Koid::alloc(),
            packet_type: PacketType::SignalOne,
        };
        dispatch.queue_packet(packet.clone());
        dispatch.queue_packet(packet);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn observer_list_with_waker_dispatch() {
        let (waker, count) = crate::test_util::counting_waker();
        let dispatch: Arc<dyn PortDispatch> = Arc::new(WakerDispatch::new(waker));
        let list = ObserverList::new();
        list.add(dispatch, 42, Signals::READABLE);
        list.notify(Signals::READABLE, Koid::alloc());
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn signal_update_fires_waker_dispatch() {
        let (waker, count) = crate::test_util::counting_waker();
        let dispatch: Arc<dyn PortDispatch> = Arc::new(WakerDispatch::new(waker));
        let signals = AtomicU32::new(0);
        let observers = ObserverList::new();
        observers.add(dispatch, 0, Signals::READABLE);
        signal_update(
            &signals,
            Signals::READABLE,
            Signals::empty(),
            &observers,
            Koid::alloc(),
        );
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;
    extern crate alloc;
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use hadron_core::sync::SpinLock;

    use crate::object::{Koid, Signals};
    use crate::port_packet::{PacketType, PortPacket};

    /// Mock port that collects delivered packets (duplicated for Kani since
    /// `#[cfg(test)]` items are not available under `#[cfg(kani)]`).
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

    #[kani::proof]
    fn kani_observer_no_lost_notification() {
        let signals: u32 = kani::any();
        kani::assume(signals != 0 && signals <= 0xFF);
        let signals = Signals::from_bits_truncate(signals);

        let mask: u32 = kani::any();
        kani::assume(mask != 0 && mask <= 0xFF);
        let mask = Signals::from_bits_truncate(mask);

        let port = MockPort::new();
        let list = ObserverList::new();
        list.add(port.clone(), 1, mask);
        list.notify(signals, Koid::alloc());

        if mask.intersects(signals) {
            assert_eq!(port.take_packets().len(), 1);
        } else {
            assert!(port.take_packets().is_empty());
        }
    }

    #[kani::proof]
    fn kani_observer_one_shot_removes() {
        let port = MockPort::new();
        let list = ObserverList::new();
        list.add(port.clone(), 1, Signals::READABLE);
        list.notify(Signals::READABLE, Koid::alloc());
        assert_eq!(port.take_packets().len(), 1);
        // Second notify should not fire (one-shot removes observer)
        list.notify(Signals::READABLE, Koid::alloc());
        assert!(port.take_packets().is_empty());
    }

    #[kani::proof]
    fn kani_signal_update_monotonic() {
        let initial: u32 = kani::any();
        kani::assume(initial <= 0xFF);
        let set: u32 = kani::any();
        kani::assume(set <= 0xFF);
        let clear: u32 = kani::any();
        kani::assume(clear <= 0xFF);

        let signals = AtomicU32::new(initial);
        let observers = ObserverList::new();
        signal_update(
            &signals,
            Signals::from_bits_truncate(set),
            Signals::from_bits_truncate(clear),
            &observers,
            Koid::alloc(),
        );
        let result = signals.load(Ordering::Relaxed);
        let expected = (initial | set) & !clear;
        assert_eq!(result, expected);
    }
}

#[cfg(shuttle)]
mod shuttle_tests {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU32, Ordering};
    use shuttle::sync::Arc as ShuttleArc;
    use shuttle::thread;

    use super::*;
    use crate::object::{Koid, Signals};
    use crate::port_packet::PortPacket;

    struct MockPort {
        count: AtomicU32,
    }
    impl MockPort {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                count: AtomicU32::new(0),
            })
        }
    }
    impl PortDispatch for MockPort {
        fn queue_packet(&self, _packet: PortPacket) {
            self.count.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn shuttle_observer_add_notify_concurrent() {
        shuttle::check_random(
            || {
                let list = ShuttleArc::new(ObserverList::new());
                let port = MockPort::new();

                let list2 = list.clone();
                let port2 = port.clone();
                let t1 = thread::spawn(move || {
                    for i in 0..3 {
                        list2.add(port2.clone(), i, Signals::READABLE);
                    }
                });

                let list3 = list.clone();
                let t2 = thread::spawn(move || {
                    for _ in 0..3 {
                        list3.notify(Signals::READABLE, Koid::alloc());
                    }
                });

                t1.join().unwrap();
                t2.join().unwrap();

                // Each observer fires at most once (one-shot), total
                // should be <= 3 observers added.
                assert!(port.count.load(Ordering::Relaxed) <= 3);
            },
            200,
        );
    }

    #[test]
    fn shuttle_observer_add_remove_concurrent() {
        shuttle::check_random(
            || {
                let list = ShuttleArc::new(ObserverList::new());
                let port = MockPort::new();

                let list2 = list.clone();
                let port2 = port.clone();
                let t1 = thread::spawn(move || {
                    list2.add(port2.clone(), 1, Signals::READABLE);
                    list2.add(port2, 2, Signals::WRITABLE);
                });

                let list3 = list.clone();
                let port3 = port.clone();
                let t2 = thread::spawn(move || {
                    list3.remove_by_port(&(port3 as Arc<dyn PortDispatch>));
                });

                t1.join().unwrap();
                t2.join().unwrap();

                // No panics = success. State depends on scheduling.
            },
            200,
        );
    }
}
