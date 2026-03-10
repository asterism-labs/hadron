//! Fifo object — fixed-element-size queue pair.
//!
//! A FIFO is a paired IPC primitive that transfers fixed-size elements between
//! two endpoints. Unlike channels, elements have a uniform size set at creation
//! and no handles can be transferred.

use alloc::collections::VecDeque;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};

/// Errors from FIFO operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FifoError {
    /// The peer endpoint has been closed.
    PeerClosed,
    /// No elements available to read.
    ShouldWait,
    /// The peer's queue is full.
    BufferFull,
    /// The data length is not a multiple of the element size.
    BadAlignment,
}

/// One endpoint of a FIFO pair.
///
/// Created via [`Fifo::create_pair`]. Elements written to one endpoint
/// appear in the other's read queue.
pub struct Fifo {
    /// Unique identifier.
    koid: Koid,
    /// Koid of the peer endpoint.
    peer_koid: Koid,
    /// Weak reference to the peer.
    peer: SpinLock<Option<Weak<Fifo>>>,
    /// Element queue (stored as raw bytes, each element is `elem_size` bytes).
    queue: SpinLock<VecDeque<Vec<u8>>>,
    /// Size of each element in bytes.
    elem_size: u32,
    /// Maximum number of elements the queue can hold (power of 2).
    elem_count: u32,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Fifo {
    /// Create a linked pair of FIFO endpoints.
    ///
    /// `elem_count` is rounded up to the next power of 2. Each element is
    /// `elem_size` bytes.
    #[must_use]
    pub fn create_pair(elem_count: u32, elem_size: u32) -> (Arc<Self>, Arc<Self>) {
        let count = elem_count.next_power_of_two();
        let koid0 = Koid::alloc();
        let koid1 = Koid::alloc();

        let f0 = Arc::new(Self {
            koid: koid0,
            peer_koid: koid1,
            peer: SpinLock::new(None),
            queue: SpinLock::new(VecDeque::new()),
            elem_size,
            elem_count: count,
            signals: AtomicU32::new(Signals::WRITABLE.bits()),
            observers: ObserverList::new(),
        });

        let f1 = Arc::new(Self {
            koid: koid1,
            peer_koid: koid0,
            peer: SpinLock::new(Some(Arc::downgrade(&f0))),
            queue: SpinLock::new(VecDeque::new()),
            elem_size,
            elem_count: count,
            signals: AtomicU32::new(Signals::WRITABLE.bits()),
            observers: ObserverList::new(),
        });

        *f0.peer.lock() = Some(Arc::downgrade(&f1));

        (f0, f1)
    }

    /// Write whole elements to the peer's queue.
    ///
    /// `data.len()` must be a multiple of `elem_size`. Returns the number of
    /// bytes written (always a multiple of `elem_size`).
    ///
    /// # Errors
    ///
    /// - [`FifoError::BadAlignment`] if data length is not element-aligned
    /// - [`FifoError::PeerClosed`] if the peer is gone
    /// - [`FifoError::BufferFull`] if no elements could be written
    pub fn write(&self, data: &[u8]) -> Result<usize, FifoError> {
        let es = self.elem_size as usize;
        if es == 0 || data.len() % es != 0 {
            return Err(FifoError::BadAlignment);
        }

        let peer = self
            .peer
            .lock()
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(FifoError::PeerClosed)?;

        let mut q = peer.queue.lock();
        let available = (peer.elem_count as usize).saturating_sub(q.len());
        if available == 0 {
            return Err(FifoError::BufferFull);
        }

        let total_elems = data.len() / es;
        let to_write = total_elems.min(available);

        for i in 0..to_write {
            let start = i * es;
            q.push_back(data[start..start + es].to_vec());
        }

        let now_full = q.len() >= peer.elem_count as usize;
        drop(q);

        signal_update(
            &peer.signals,
            Signals::READABLE,
            Signals::empty(),
            &peer.observers,
            peer.koid,
        );

        if now_full {
            self.signals
                .fetch_and(!Signals::WRITABLE.bits(), Ordering::Release);
        }

        Ok(to_write * es)
    }

    /// Read whole elements from this endpoint's queue.
    ///
    /// `buf.len()` must be a multiple of `elem_size`. Returns the number of
    /// bytes read.
    ///
    /// # Errors
    ///
    /// - [`FifoError::BadAlignment`] if buffer length is not element-aligned
    /// - [`FifoError::ShouldWait`] if no elements are available
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, FifoError> {
        let es = self.elem_size as usize;
        if es == 0 || buf.len() % es != 0 {
            return Err(FifoError::BadAlignment);
        }

        let mut q = self.queue.lock();
        if q.is_empty() {
            return Err(FifoError::ShouldWait);
        }

        let max_elems = buf.len() / es;
        let to_read = max_elems.min(q.len());
        let was_full = q.len() >= self.elem_count as usize;

        for i in 0..to_read {
            let elem = q.pop_front().unwrap();
            buf[i * es..(i + 1) * es].copy_from_slice(&elem);
        }

        let is_empty = q.is_empty();
        drop(q);

        if is_empty {
            self.signals
                .fetch_and(!Signals::READABLE.bits(), Ordering::Release);
        }

        if was_full {
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

        Ok(to_read * es)
    }

    /// The element size in bytes.
    #[must_use]
    pub fn elem_size(&self) -> u32 {
        self.elem_size
    }

    /// The maximum number of elements (power-of-2 capacity).
    #[must_use]
    pub fn elem_count(&self) -> u32 {
        self.elem_count
    }

    /// Number of elements currently queued for reading.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.queue.lock().len()
    }
}

impl KernelObject for Fifo {
    fn object_type(&self) -> ObjectType {
        ObjectType::Fifo
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
    fn fifo_create_pair() {
        let (f0, f1) = Fifo::create_pair(5, 4);
        assert_eq!(f0.object_type(), ObjectType::Fifo);
        assert_eq!(f0.related_koid(), f1.koid());
        // 5 rounds up to 8 (next power of 2).
        assert_eq!(f0.elem_count(), 8);
        assert_eq!(f0.elem_size(), 4);
    }

    #[test]
    fn fifo_write_and_read() {
        let (f0, f1) = Fifo::create_pair(8, 4);

        // Write two 4-byte elements.
        let data = [1u8, 0, 0, 0, 2, 0, 0, 0];
        let written = f0.write(&data).unwrap();
        assert_eq!(written, 8);

        let mut buf = [0u8; 8];
        let read = f1.read(&mut buf).unwrap();
        assert_eq!(read, 8);
        assert_eq!(buf, data);
    }

    #[test]
    fn fifo_bad_alignment() {
        let (f0, _f1) = Fifo::create_pair(8, 4);
        // 3 bytes is not a multiple of 4.
        assert_eq!(f0.write(&[1, 2, 3]), Err(FifoError::BadAlignment));
    }

    #[test]
    fn fifo_capacity_limit() {
        let (f0, f1) = Fifo::create_pair(2, 1);
        // Capacity = 2 (already power of 2).
        assert_eq!(f0.elem_count(), 2);

        f0.write(&[1, 2]).unwrap();
        assert_eq!(f0.write(&[3]), Err(FifoError::BufferFull));

        let mut buf = [0u8; 2];
        f1.read(&mut buf).unwrap();
        assert_eq!(buf, [1, 2]);
    }

    #[test]
    fn fifo_peer_closed() {
        let (f0, f1) = Fifo::create_pair(8, 4);
        f0.on_zero_handles();
        drop(f0);

        assert!(f1.get_signals().contains(Signals::PEER_CLOSED));
    }

    #[test]
    fn fifo_partial_write() {
        let (f0, f1) = Fifo::create_pair(2, 4);
        // Try to write 3 elements into capacity-2 queue.
        let data = [0u8; 12]; // 3 elements of 4 bytes.
        let written = f0.write(&data).unwrap();
        assert_eq!(written, 8); // Only 2 elements fit.
        assert_eq!(f1.pending(), 2);
    }
}
