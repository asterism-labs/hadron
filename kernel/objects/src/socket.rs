//! Socket object — streaming byte IPC pair.
//!
//! A socket is a paired, bidirectional byte stream. Unlike channels, sockets
//! carry no message boundaries and cannot transfer handles. Partial reads
//! and writes are supported.

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};

/// Default maximum buffer size per socket endpoint (256 KiB).
pub const DEFAULT_MAX_BUFFER: usize = 256 * 1024;

/// Errors from socket operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketError {
    /// The peer endpoint has been closed.
    PeerClosed,
    /// No data available to read.
    ShouldWait,
    /// The peer's buffer is full; no bytes could be written.
    BufferFull,
}

/// One endpoint of a socket pair.
///
/// Created via [`Socket::create_pair`]. Bytes written to one endpoint can be
/// read from the other. No message framing — this is a raw byte stream.
pub struct Socket {
    /// Unique identifier.
    koid: Koid,
    /// Koid of the peer endpoint.
    peer_koid: Koid,
    /// Weak reference to the peer.
    peer: SpinLock<Option<Weak<Socket>>>,
    /// Incoming byte buffer (bytes written by the peer).
    buffer: SpinLock<VecDeque<u8>>,
    /// Maximum buffer capacity in bytes.
    max_buffer: usize,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Socket {
    /// Create a linked pair of socket endpoints.
    #[must_use]
    pub fn create_pair(max_buffer: usize) -> (Arc<Self>, Arc<Self>) {
        let koid0 = Koid::alloc();
        let koid1 = Koid::alloc();

        let s0 = Arc::new(Self {
            koid: koid0,
            peer_koid: koid1,
            peer: SpinLock::new(None),
            buffer: SpinLock::new(VecDeque::new()),
            max_buffer,
            signals: AtomicU32::new(Signals::WRITABLE.bits()),
            observers: ObserverList::new(),
        });

        let s1 = Arc::new(Self {
            koid: koid1,
            peer_koid: koid0,
            peer: SpinLock::new(Some(Arc::downgrade(&s0))),
            buffer: SpinLock::new(VecDeque::new()),
            max_buffer,
            signals: AtomicU32::new(Signals::WRITABLE.bits()),
            observers: ObserverList::new(),
        });

        *s0.peer.lock() = Some(Arc::downgrade(&s1));

        (s0, s1)
    }

    /// Write bytes to the peer's buffer.
    ///
    /// Returns the number of bytes actually written (may be less than
    /// `data.len()` if the peer's buffer fills up).
    ///
    /// # Errors
    ///
    /// - [`SocketError::PeerClosed`] if the peer is gone
    /// - [`SocketError::BufferFull`] if the peer's buffer is completely full
    pub fn write(&self, data: &[u8]) -> Result<usize, SocketError> {
        let peer = self
            .peer
            .lock()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(SocketError::PeerClosed)?;

        let mut buf = peer.buffer.lock();
        let available = peer.max_buffer.saturating_sub(buf.len());
        if available == 0 {
            return Err(SocketError::BufferFull);
        }

        let to_write = data.len().min(available);
        buf.extend(&data[..to_write]);

        let now_full = buf.len() >= peer.max_buffer;
        drop(buf);

        // Assert READABLE on peer.
        signal_update(
            &peer.signals,
            Signals::READABLE,
            Signals::empty(),
            &peer.observers,
            peer.koid,
        );

        // If peer buffer is now full, clear WRITABLE on self.
        if now_full {
            self.signals
                .fetch_and(!Signals::WRITABLE.bits(), Ordering::Release);
        }

        Ok(to_write)
    }

    /// Read bytes from this endpoint's buffer.
    ///
    /// Returns the number of bytes read into `buf`.
    ///
    /// # Errors
    ///
    /// Returns [`SocketError::ShouldWait`] if no data is available.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, SocketError> {
        let mut internal = self.buffer.lock();
        if internal.is_empty() {
            return Err(SocketError::ShouldWait);
        }

        let to_read = buf.len().min(internal.len());
        for byte in buf.iter_mut().take(to_read) {
            *byte = internal.pop_front().unwrap();
        }

        let was_full_before = internal.len() + to_read >= self.max_buffer;
        let is_empty = internal.is_empty();
        drop(internal);

        if is_empty {
            self.signals
                .fetch_and(!Signals::READABLE.bits(), Ordering::Release);
        }

        // If we freed space and peer exists, re-assert WRITABLE on peer.
        if was_full_before {
            let peer = self.peer.lock().as_ref().and_then(Weak::upgrade);
            if let Some(p) = peer {
                signal_update(
                    &p.signals,
                    Signals::WRITABLE,
                    Signals::empty(),
                    &p.observers,
                    p.koid,
                );
            }
        }

        Ok(to_read)
    }

    /// Number of bytes buffered for reading on this endpoint.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.buffer.lock().len()
    }
}

impl KernelObject for Socket {
    fn object_type(&self) -> ObjectType {
        ObjectType::Socket
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
    use super::*;

    #[test]
    fn socket_create_pair() {
        let (s0, s1) = Socket::create_pair(DEFAULT_MAX_BUFFER);
        assert_eq!(s0.object_type(), ObjectType::Socket);
        assert_eq!(s0.related_koid(), s1.koid());
        assert_eq!(s1.related_koid(), s0.koid());
    }

    #[test]
    fn socket_write_and_read() {
        let (s0, s1) = Socket::create_pair(DEFAULT_MAX_BUFFER);

        let written = s0.write(b"hello").unwrap();
        assert_eq!(written, 5);
        assert!(s1.get_signals().contains(Signals::READABLE));

        let mut buf = [0u8; 16];
        let read = s1.read(&mut buf).unwrap();
        assert_eq!(read, 5);
        assert_eq!(&buf[..5], b"hello");
    }

    #[test]
    fn socket_partial_read() {
        let (s0, s1) = Socket::create_pair(DEFAULT_MAX_BUFFER);
        s0.write(b"hello world").unwrap();

        let mut buf = [0u8; 5];
        let read = s1.read(&mut buf).unwrap();
        assert_eq!(read, 5);
        assert_eq!(&buf, b"hello");

        // Remaining data still readable.
        assert!(s1.get_signals().contains(Signals::READABLE));
        let read2 = s1.read(&mut buf).unwrap();
        assert_eq!(read2, 5);
        assert_eq!(&buf, b" worl");
    }

    #[test]
    fn socket_buffer_full() {
        let (s0, s1) = Socket::create_pair(8);
        s0.write(b"12345678").unwrap();

        // Buffer is full — next write should fail.
        assert_eq!(s0.write(b"x"), Err(SocketError::BufferFull));
        assert!(!s0.get_signals().contains(Signals::WRITABLE));

        // Read some to free space.
        let mut buf = [0u8; 4];
        s1.read(&mut buf).unwrap();

        // WRITABLE should be re-asserted.
        assert!(s0.get_signals().contains(Signals::WRITABLE));
    }

    #[test]
    fn socket_partial_write() {
        let (s0, s1) = Socket::create_pair(8);
        let written = s0.write(b"1234567890").unwrap();
        assert_eq!(written, 8); // Only 8 bytes fit.
        assert_eq!(s1.buffered(), 8);
    }

    #[test]
    fn socket_peer_closed() {
        let (s0, s1) = Socket::create_pair(DEFAULT_MAX_BUFFER);
        s0.on_zero_handles();
        drop(s0);

        assert!(s1.get_signals().contains(Signals::PEER_CLOSED));
        assert!(!s1.get_signals().contains(Signals::WRITABLE));
        assert_eq!(s1.write(b"fail"), Err(SocketError::PeerClosed));
    }

    #[test]
    fn socket_read_empty() {
        let (_s0, s1) = Socket::create_pair(DEFAULT_MAX_BUFFER);
        let mut buf = [0u8; 16];
        assert_eq!(s1.read(&mut buf), Err(SocketError::ShouldWait));
    }

    #[test]
    fn socket_bidirectional() {
        let (s0, s1) = Socket::create_pair(DEFAULT_MAX_BUFFER);
        s0.write(b"ping").unwrap();
        s1.write(b"pong").unwrap();

        let mut buf = [0u8; 4];
        s1.read(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
        s0.read(&mut buf).unwrap();
        assert_eq!(&buf, b"pong");
    }
}
