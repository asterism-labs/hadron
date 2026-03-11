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
}
