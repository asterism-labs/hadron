//! Port packet types for async event delivery.
//!
//! A [`PortPacket`] represents a single notification delivered to a [`Port`](super::port::Port).
//! Packets are created by the observer infrastructure when signals change, or
//! explicitly by userspace via `port_queue`.

use crate::object::{Koid, Signals};

/// The type of event that generated a port packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    /// Delivered by a one-shot signal observer (`object_wait_async`).
    SignalOne,
    /// Queued explicitly by userspace (`port_queue`).
    User,
}

/// A single notification delivered to a port.
///
/// Each packet carries enough context for the receiver to identify which
/// object changed, what signals are now asserted, and the caller-supplied
/// key for demultiplexing.
#[derive(Debug, Clone)]
pub struct PortPacket {
    /// Caller-supplied key for demultiplexing multiple waits on one port.
    pub key: u64,
    /// The signal state at the time of delivery.
    pub signals: Signals,
    /// The koid of the object that generated this packet.
    pub koid: Koid,
    /// How this packet was generated.
    pub packet_type: PacketType,
}
