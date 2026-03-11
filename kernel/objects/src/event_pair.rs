//! EventPair object — linked pair of signaling primitives.
//!
//! An event pair consists of two peer objects. Each side can set signals on
//! the other, and closing one side asserts `PEER_CLOSED` on the surviving peer.

use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};

/// One end of an event pair.
///
/// Created via [`EventPair::create_pair`], which returns both endpoints.
/// Each endpoint can signal the other, and closing one asserts `PEER_CLOSED`
/// on the surviving peer.
pub struct EventPair {
    /// Unique identifier for this endpoint.
    koid: Koid,
    /// Koid of the peer endpoint.
    peer_koid: Koid,
    /// Weak reference to the peer (avoids Arc cycle).
    peer: SpinLock<Option<Weak<EventPair>>>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl EventPair {
    /// Create a linked pair of event pair endpoints.
    #[must_use]
    pub fn create_pair() -> (Arc<Self>, Arc<Self>) {
        let koid0 = Koid::alloc();
        let koid1 = Koid::alloc();

        let ep0 = Arc::new(Self {
            koid: koid0,
            peer_koid: koid1,
            peer: SpinLock::new(None),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        });

        let ep1 = Arc::new(Self {
            koid: koid1,
            peer_koid: koid0,
            peer: SpinLock::new(Some(Arc::downgrade(&ep0))),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        });

        *ep0.peer.lock() = Some(Arc::downgrade(&ep1));

        (ep0, ep1)
    }

    /// Set and/or clear signals on this endpoint.
    pub fn signal(&self, set: Signals, clear: Signals) {
        signal_update(&self.signals, set, clear, &self.observers, self.koid);
    }

    /// Signal the peer endpoint.
    ///
    /// Returns `false` if the peer is already closed.
    pub fn signal_peer(&self, set: Signals, clear: Signals) -> bool {
        let peer = self.peer.lock().as_ref().and_then(Weak::upgrade);
        match peer {
            Some(p) => {
                signal_update(&p.signals, set, clear, &p.observers, p.koid);
                true
            }
            None => false,
        }
    }
}

impl KernelObject for EventPair {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::EventPair
    }

    fn koid(&self) -> Koid {
        self.koid
    }

    fn related_koid(&self) -> Koid {
        self.peer_koid
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

    fn on_zero_handles(&self) {
        // Assert PEER_CLOSED on the surviving peer.
        let peer = self.peer.lock().as_ref().and_then(Weak::upgrade);
        if let Some(p) = peer {
            signal_update(
                &p.signals,
                Signals::PEER_CLOSED,
                Signals::empty(),
                &p.observers,
                p.koid,
            );
        }
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
    fn create_pair_linked() {
        let (ep0, ep1) = EventPair::create_pair();
        assert_eq!(ep0.related_koid(), ep1.koid());
        assert_eq!(ep1.related_koid(), ep0.koid());
        assert_eq!(ep0.object_type(), ObjectType::EventPair);
    }

    #[test]
    fn signal_self() {
        let (ep0, _ep1) = EventPair::create_pair();
        ep0.signal(Signals::SIGNAL_0, Signals::empty());
        assert!(ep0.get_signals().contains(Signals::SIGNAL_0));
    }

    #[test]
    fn signal_peer() {
        let (ep0, ep1) = EventPair::create_pair();
        assert!(ep0.signal_peer(Signals::SIGNAL_0, Signals::empty()));
        assert!(ep1.get_signals().contains(Signals::SIGNAL_0));
    }

    #[test]
    fn peer_closed_on_drop() {
        let (ep0, ep1) = EventPair::create_pair();
        let port = MockPort::new();
        ep1.add_observer(port.clone(), 1, Signals::PEER_CLOSED);

        // Simulate closing ep0: call on_zero_handles then drop.
        ep0.on_zero_handles();
        drop(ep0);

        assert!(ep1.get_signals().contains(Signals::PEER_CLOSED));
        let packets = port.take_packets();
        assert_eq!(packets.len(), 1);
        assert!(packets[0].signals.contains(Signals::PEER_CLOSED));
    }

    #[test]
    fn signal_peer_after_close_returns_false() {
        let (ep0, ep1) = EventPair::create_pair();
        ep0.on_zero_handles();
        drop(ep0);

        assert!(!ep1.signal_peer(Signals::SIGNAL_0, Signals::empty()));
    }
}
