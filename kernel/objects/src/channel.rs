//! Channel object — bidirectional message + handle passing IPC.
//!
//! A channel is a paired, bidirectional message transport. Messages can carry
//! both byte data (up to 64 KiB) and handles (up to 64). Closing one endpoint
//! asserts `PEER_CLOSED` on the other.

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::handle::HandleEntry;
use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};

/// Maximum byte payload per message.
pub const MAX_MSG_DATA: usize = 65536;

/// Maximum number of handles per message.
pub const MAX_MSG_HANDLES: usize = 64;

/// A single message in a channel queue.
#[derive(Debug)]
pub struct ChannelMessage {
    /// Byte payload.
    pub data: Vec<u8>,
    /// Handles transferred with this message.
    pub handles: Vec<HandleEntry>,
}

/// Errors from channel operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelError {
    /// Message data exceeds [`MAX_MSG_DATA`].
    MessageTooLarge,
    /// Message carries too many handles (> [`MAX_MSG_HANDLES`]).
    TooManyHandles,
    /// The peer endpoint has been closed.
    PeerClosed,
    /// No messages available to read.
    ShouldWait,
}

/// One endpoint of a channel pair.
///
/// Created via [`Channel::create_pair`]. Messages written to one endpoint
/// appear in the other's read queue.
pub struct Channel {
    /// Unique identifier for this endpoint.
    koid: Koid,
    /// Koid of the peer endpoint.
    peer_koid: Koid,
    /// Weak reference to the peer (avoids Arc cycle).
    peer: SpinLock<Option<Weak<Channel>>>,
    /// Incoming message queue (messages written by the peer).
    messages: SpinLock<VecDeque<ChannelMessage>>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Channel {
    /// Create a linked pair of channel endpoints.
    #[must_use]
    pub fn create_pair() -> (Arc<Self>, Arc<Self>) {
        let koid0 = Koid::alloc();
        let koid1 = Koid::alloc();

        let ch0 = Arc::new(Self {
            koid: koid0,
            peer_koid: koid1,
            peer: SpinLock::new(None),
            messages: SpinLock::new(VecDeque::new()),
            signals: AtomicU32::new(Signals::WRITABLE.bits()),
            observers: ObserverList::new(),
        });

        let ch1 = Arc::new(Self {
            koid: koid1,
            peer_koid: koid0,
            peer: SpinLock::new(Some(Arc::downgrade(&ch0))),
            messages: SpinLock::new(VecDeque::new()),
            signals: AtomicU32::new(Signals::WRITABLE.bits()),
            observers: ObserverList::new(),
        });

        *ch0.peer.lock() = Some(Arc::downgrade(&ch1));

        (ch0, ch1)
    }

    /// Write a message to the peer's read queue.
    ///
    /// # Errors
    ///
    /// - [`ChannelError::MessageTooLarge`] if data exceeds 64 KiB
    /// - [`ChannelError::TooManyHandles`] if handles exceed 64
    /// - [`ChannelError::PeerClosed`] if the peer endpoint is gone
    pub fn write(&self, msg: ChannelMessage) -> Result<(), ChannelError> {
        if msg.data.len() > MAX_MSG_DATA {
            return Err(ChannelError::MessageTooLarge);
        }
        if msg.handles.len() > MAX_MSG_HANDLES {
            return Err(ChannelError::TooManyHandles);
        }

        let peer = self
            .peer
            .lock()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(ChannelError::PeerClosed)?;

        peer.messages.lock().push_back(msg);

        // Assert READABLE on the peer.
        signal_update(
            &peer.signals,
            Signals::READABLE,
            Signals::empty(),
            &peer.observers,
            peer.koid,
        );

        Ok(())
    }

    /// Read a message from this endpoint's queue.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelError::ShouldWait`] if no messages are available.
    pub fn read(&self) -> Result<ChannelMessage, ChannelError> {
        let mut q = self.messages.lock();
        match q.pop_front() {
            Some(msg) => {
                if q.is_empty() {
                    drop(q);
                    // Clear READABLE when queue drains.
                    self.signals
                        .fetch_and(!Signals::READABLE.bits(), Ordering::Release);
                }
                Ok(msg)
            }
            None => Err(ChannelError::ShouldWait),
        }
    }

    /// Number of messages in this endpoint's read queue.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.messages.lock().len()
    }
}

impl KernelObject for Channel {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Channel
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
                Signals::WRITABLE,
                &p.observers,
                p.koid,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn make_msg(data: &[u8]) -> ChannelMessage {
        ChannelMessage {
            data: data.to_vec(),
            handles: Vec::new(),
        }
    }

    #[test]
    fn channel_create_pair() {
        let (ch0, ch1) = Channel::create_pair();
        assert_eq!(ch0.object_type(), ObjectType::Channel);
        assert_eq!(ch0.related_koid(), ch1.koid());
        assert_eq!(ch1.related_koid(), ch0.koid());
    }

    #[test]
    fn channel_write_and_read() {
        let (ch0, ch1) = Channel::create_pair();

        ch0.write(make_msg(b"hello")).unwrap();
        assert!(ch1.get_signals().contains(Signals::READABLE));

        let msg = ch1.read().unwrap();
        assert_eq!(msg.data, b"hello");
        assert!(!ch1.get_signals().contains(Signals::READABLE));
    }

    #[test]
    fn channel_fifo_ordering() {
        let (ch0, ch1) = Channel::create_pair();
        ch0.write(make_msg(b"first")).unwrap();
        ch0.write(make_msg(b"second")).unwrap();

        assert_eq!(ch1.read().unwrap().data, b"first");
        assert_eq!(ch1.read().unwrap().data, b"second");
    }

    #[test]
    fn channel_bidirectional() {
        let (ch0, ch1) = Channel::create_pair();
        ch0.write(make_msg(b"ping")).unwrap();
        ch1.write(make_msg(b"pong")).unwrap();

        assert_eq!(ch1.read().unwrap().data, b"ping");
        assert_eq!(ch0.read().unwrap().data, b"pong");
    }

    #[test]
    fn channel_message_too_large() {
        let (ch0, _ch1) = Channel::create_pair();
        let big = vec![0u8; MAX_MSG_DATA + 1];
        assert!(matches!(
            ch0.write(make_msg(&big)),
            Err(ChannelError::MessageTooLarge)
        ));
    }

    #[test]
    fn channel_too_many_handles() {
        let (ch0, _ch1) = Channel::create_pair();
        let handles: Vec<HandleEntry> = (0..MAX_MSG_HANDLES + 1)
            .map(|_| {
                let obj = crate::event::Event::new();
                HandleEntry::new(obj, crate::handle::Rights::ALL)
            })
            .collect();
        let msg = ChannelMessage {
            data: Vec::new(),
            handles,
        };
        assert!(matches!(ch0.write(msg), Err(ChannelError::TooManyHandles)));
    }

    #[test]
    fn channel_peer_closed() {
        let (ch0, ch1) = Channel::create_pair();
        ch0.on_zero_handles();
        drop(ch0);

        assert!(ch1.get_signals().contains(Signals::PEER_CLOSED));
        assert!(!ch1.get_signals().contains(Signals::WRITABLE));
        assert!(matches!(
            ch1.write(make_msg(b"fail")),
            Err(ChannelError::PeerClosed)
        ));
    }

    #[test]
    fn channel_read_empty() {
        let (_ch0, ch1) = Channel::create_pair();
        assert!(matches!(ch1.read(), Err(ChannelError::ShouldWait)));
    }

    #[test]
    fn channel_max_data_ok() {
        let (ch0, ch1) = Channel::create_pair();
        let data = vec![0xAB; MAX_MSG_DATA];
        ch0.write(make_msg(&data)).unwrap();
        assert_eq!(ch1.read().unwrap().data.len(), MAX_MSG_DATA);
    }
}
