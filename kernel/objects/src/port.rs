//! Port object — async event aggregator.
//!
//! A port collects [`PortPacket`]s from observers and explicit user queuing.
//! Userspace calls `port_wait` to dequeue the next packet, blocking if the
//! queue is empty. Observers on other objects deliver packets here when
//! signals of interest change.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};
use crate::port_packet::{PacketType, PortPacket};

/// Errors from port operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortError {
    /// The port queue is empty and the deadline has elapsed.
    TimedOut,
}

/// A port — the async event aggregation primitive.
///
/// Ports are the central mechanism for waiting on multiple kernel objects
/// simultaneously. Each port maintains a FIFO queue of [`PortPacket`]s.
/// Observers registered on other objects deliver packets here when signals
/// change. Userspace can also explicitly queue packets via `port_queue`.
pub struct Port {
    /// Unique identifier.
    koid: Koid,
    /// FIFO queue of pending packets.
    queue: SpinLock<VecDeque<PortPacket>>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers on this port itself.
    observers: ObserverList,
}

impl Port {
    /// Create a new empty port.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            queue: SpinLock::new(VecDeque::new()),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Try to dequeue a packet without blocking.
    ///
    /// Returns `Ok(packet)` if the queue is non-empty, or `Err(PortError::TimedOut)`
    /// if empty. A full blocking `port_wait` with deadline support requires
    /// integration with the kernel's async executor (out of scope for this object).
    pub fn try_wait(&self) -> Result<PortPacket, PortError> {
        let mut q = self.queue.lock();
        match q.pop_front() {
            Some(packet) => {
                if q.is_empty() {
                    // Clear READABLE when queue drains.
                    self.signals
                        .fetch_and(!Signals::READABLE.bits(), Ordering::Release);
                }
                Ok(packet)
            }
            None => Err(PortError::TimedOut),
        }
    }

    /// Queue an explicit user packet.
    pub fn queue_user_packet(&self, key: u64, signals: Signals) {
        let packet = PortPacket {
            key,
            signals,
            koid: self.koid,
            packet_type: PacketType::User,
        };
        self.enqueue(packet);
    }

    /// Internal: push a packet and assert READABLE.
    fn enqueue(&self, packet: PortPacket) {
        self.queue.lock().push_back(packet);
        signal_update(
            &self.signals,
            Signals::READABLE,
            Signals::empty(),
            &self.observers,
            self.koid,
        );
    }

    /// Number of packets currently in the queue.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.queue.lock().len()
    }
}

impl PortDispatch for Port {
    fn queue_packet(&self, packet: PortPacket) {
        self.enqueue(packet);
    }
}

impl KernelObject for Port {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Port
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
    use super::*;

    #[test]
    fn port_create_and_properties() {
        let port = Port::new();
        assert_eq!(port.object_type(), ObjectType::Port);
        assert_eq!(port.pending_count(), 0);
    }

    #[test]
    fn port_queue_and_wait() {
        let port = Port::new();
        port.queue_user_packet(42, Signals::SIGNAL_0);

        assert!(port.get_signals().contains(Signals::READABLE));
        assert_eq!(port.pending_count(), 1);

        let packet = port.try_wait().expect("should have a packet");
        assert_eq!(packet.key, 42);
        assert_eq!(packet.packet_type, PacketType::User);

        // Queue is now empty — READABLE should be cleared.
        assert!(!port.get_signals().contains(Signals::READABLE));
        assert!(port.try_wait().is_err());
    }

    #[test]
    fn port_receives_observer_packets() {
        let port = Port::new();
        let observers = ObserverList::new();
        let koid = Koid::alloc();

        // Register port as observer.
        observers.add(port.clone() as Arc<dyn PortDispatch>, 99, Signals::READABLE);

        // Fire the observer.
        observers.notify(Signals::READABLE, koid);

        let packet = port.try_wait().expect("should have observer packet");
        assert_eq!(packet.key, 99);
        assert_eq!(packet.koid, koid);
        assert_eq!(packet.packet_type, PacketType::SignalOne);
    }

    #[test]
    fn port_fifo_ordering() {
        let port = Port::new();
        port.queue_user_packet(1, Signals::SIGNAL_0);
        port.queue_user_packet(2, Signals::SIGNAL_1);
        port.queue_user_packet(3, Signals::SIGNAL_2);

        assert_eq!(port.try_wait().unwrap().key, 1);
        assert_eq!(port.try_wait().unwrap().key, 2);
        assert_eq!(port.try_wait().unwrap().key, 3);
    }
}
