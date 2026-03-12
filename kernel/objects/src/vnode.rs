//! Vnode kernel object — represents an open file backed by a per-file channel.
//!
//! Each vnode wraps a channel endpoint connected to the filesystem server that
//! owns the underlying file. All file operations (read, write, stat, readdir)
//! are forwarded as messages on this channel. Closing the last handle to the
//! vnode triggers `PEER_CLOSED` on the server end, implicitly closing the file.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::channel::Channel;
use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};

/// A vnode — an open file or directory backed by a per-file channel pair.
///
/// The kernel holds one end of the channel; the FS server holds the other.
/// Seek state is tracked kernel-side so `SEEK_SET` / `SEEK_CUR` avoid
/// a server roundtrip.
pub struct Vnode {
    /// Unique identifier for this vnode.
    koid: Koid,
    /// Kernel's end of the per-file channel pair.
    channel: Arc<Channel>,
    /// Open flags (`OPEN_RDONLY`, `OPEN_WRONLY`, `OPEN_RDWR`, `OPEN_DIRECTORY`, etc.).
    open_flags: u32,
    /// Current seek offset, updated by read/write/seek operations.
    seek_offset: SpinLock<u64>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Vnode {
    /// Create a new vnode wrapping the given per-file channel endpoint.
    #[must_use]
    pub fn new(channel: Arc<Channel>, open_flags: u32) -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            channel,
            open_flags,
            seek_offset: SpinLock::new(0),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// The per-file channel endpoint (kernel side).
    #[must_use]
    pub fn channel(&self) -> &Arc<Channel> {
        &self.channel
    }

    /// The open flags for this vnode.
    #[must_use]
    pub fn open_flags(&self) -> u32 {
        self.open_flags
    }

    /// Read the current seek offset.
    #[must_use]
    pub fn seek_offset(&self) -> u64 {
        *self.seek_offset.lock()
    }

    /// Set the seek offset to an absolute value.
    pub fn set_seek_offset(&self, offset: u64) {
        *self.seek_offset.lock() = offset;
    }

    /// Advance the seek offset by `delta` bytes.
    pub fn advance_seek_offset(&self, delta: u64) {
        let mut off = self.seek_offset.lock();
        *off = off.saturating_add(delta);
    }

    /// Set and/or clear signals on this vnode.
    pub fn signal(&self, set: Signals, clear: Signals) {
        signal_update(&self.signals, set, clear, &self.observers, self.koid);
    }
}

impl KernelObject for Vnode {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Vnode
    }

    fn koid(&self) -> Koid {
        self.koid
    }

    fn related_koid(&self) -> Koid {
        self.channel.koid()
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
        // Propagate to the inner channel so the server sees PEER_CLOSED.
        self.channel.on_zero_handles();
    }
}

#[cfg(test)]
mod tests {
    use alloc::sync::Arc;
    use alloc::vec::Vec;

    use hadron_core::sync::SpinLock;

    use super::*;
    use crate::channel::Channel;
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
    fn vnode_create_and_properties() {
        let (ch0, _ch1) = Channel::create_pair();
        let vnode = Vnode::new(ch0, 0);
        assert_eq!(vnode.object_type(), ObjectType::Vnode);
        assert_eq!(vnode.open_flags(), 0);
        assert_eq!(vnode.seek_offset(), 0);
        assert_eq!(vnode.get_signals(), Signals::empty());
    }

    #[test]
    fn vnode_seek_offset_operations() {
        let (ch0, _ch1) = Channel::create_pair();
        let vnode = Vnode::new(ch0, 0);

        vnode.set_seek_offset(100);
        assert_eq!(vnode.seek_offset(), 100);

        vnode.advance_seek_offset(50);
        assert_eq!(vnode.seek_offset(), 150);
    }

    #[test]
    fn vnode_on_zero_handles_closes_channel() {
        let (ch0, ch1) = Channel::create_pair();
        let vnode = Vnode::new(ch0, 0);

        vnode.on_zero_handles();

        // ch1 should see PEER_CLOSED.
        assert!(ch1.get_signals().contains(Signals::PEER_CLOSED));
    }

    #[test]
    fn vnode_related_koid_is_channel_koid() {
        let (ch0, _ch1) = Channel::create_pair();
        let ch0_koid = ch0.koid();
        let vnode = Vnode::new(ch0, 0);
        assert_eq!(vnode.related_koid(), ch0_koid);
    }

    #[test]
    fn vnode_notifies_observer() {
        let (ch0, _ch1) = Channel::create_pair();
        let vnode = Vnode::new(ch0, 0);
        let port = MockPort::new();

        vnode.add_observer(port.clone(), 42, Signals::SIGNAL_0);
        vnode.signal(Signals::SIGNAL_0, Signals::empty());

        let packets = port.take_packets();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].key, 42);
    }
}
