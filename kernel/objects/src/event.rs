//! Event object — simple signal holder.
//!
//! An event is a lightweight signaling primitive. Any holder with
//! [`Rights::SIGNAL`](crate::handle::Rights::SIGNAL) can set or clear
//! user-visible signals. Observers are notified on signal changes.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};

/// An event — a pure signal-holding object.
///
/// Events carry no data; they exist solely so userspace can wait on and
/// signal arbitrary conditions. Use [`EventPair`](super::event_pair::EventPair)
/// when two-party coordination with `PEER_CLOSED` is needed.
pub struct Event {
    /// Unique identifier.
    koid: Koid,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Event {
    /// Create a new event with no signals asserted.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Set and/or clear signals on this event.
    ///
    /// Bits in `set` are asserted; bits in `clear` are deasserted.
    pub fn signal(&self, set: Signals, clear: Signals) {
        signal_update(&self.signals, set, clear, &self.observers, self.koid);
    }
}

impl KernelObject for Event {
    fn object_type(&self) -> ObjectType {
        ObjectType::Event
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
    fn event_create_and_properties() {
        let event = Event::new();
        assert_eq!(event.object_type(), ObjectType::Event);
        assert_eq!(event.get_signals(), Signals::empty());
    }

    #[test]
    fn event_set_and_clear_signals() {
        let event = Event::new();
        event.signal(Signals::SIGNAL_0 | Signals::SIGNAL_1, Signals::empty());
        assert!(event.get_signals().contains(Signals::SIGNAL_0));
        assert!(event.get_signals().contains(Signals::SIGNAL_1));

        event.signal(Signals::empty(), Signals::SIGNAL_0);
        assert!(!event.get_signals().contains(Signals::SIGNAL_0));
        assert!(event.get_signals().contains(Signals::SIGNAL_1));
    }

    #[test]
    fn event_notifies_observer() {
        let event = Event::new();
        let port = MockPort::new();

        event.add_observer(port.clone(), 7, Signals::SIGNAL_0);
        event.signal(Signals::SIGNAL_0, Signals::empty());

        let packets = port.take_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].key, 7);
        assert_eq!(packets[0].koid, event.koid());
    }
}
