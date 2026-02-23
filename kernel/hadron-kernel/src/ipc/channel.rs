//! Bidirectional message-oriented channel for IPC.
//!
//! A channel has two endpoints (A and B). Messages sent on endpoint A are
//! received on endpoint B, and vice versa. Unlike pipes, channels carry
//! discrete messages (not a byte stream) and are bidirectional.
//!
//! Follows the same async patterns as [`super::pipe`]: register-before-check
//! wakeups, `HeapWaitQueue`, and `SpinLock`-protected queues.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};
use crate::sync::{HeapWaitQueue, SpinLock};

/// Maximum size of a single channel message in bytes.
const MAX_MESSAGE_SIZE: usize = 4096;

/// Maximum number of buffered messages per direction before blocking.
const MAX_BUFFERED_MESSAGES: usize = 16;

/// Creates a new channel, returning the two endpoints as `Arc<dyn Inode>`.
pub fn channel() -> (Arc<dyn Inode>, Arc<dyn Inode>) {
    let inner = Arc::new(ChannelInner {
        a_to_b: SpinLock::named("channel_a2b", alloc::collections::VecDeque::new()),
        b_to_a: SpinLock::named("channel_b2a", alloc::collections::VecDeque::new()),
        a_send_wq: HeapWaitQueue::new(),
        a_recv_wq: HeapWaitQueue::new(),
        b_send_wq: HeapWaitQueue::new(),
        b_recv_wq: HeapWaitQueue::new(),
        a_endpoints: AtomicUsize::new(1),
        b_endpoints: AtomicUsize::new(1),
    });
    let endpoint_a = Arc::new(ChannelEndpoint {
        inner: inner.clone(),
        is_a: true,
    });
    let endpoint_b = Arc::new(ChannelEndpoint { inner, is_a: false });
    (endpoint_a, endpoint_b)
}

/// Shared channel state between both endpoints.
struct ChannelInner {
    /// Messages sent by A, received by B.
    a_to_b: SpinLock<alloc::collections::VecDeque<Vec<u8>>>,
    /// Messages sent by B, received by A.
    b_to_a: SpinLock<alloc::collections::VecDeque<Vec<u8>>>,
    /// Woken when A can send (space in `a_to_b`).
    a_send_wq: HeapWaitQueue,
    /// Woken when A can recv (data in `b_to_a`).
    a_recv_wq: HeapWaitQueue,
    /// Woken when B can send (space in `b_to_a`).
    b_send_wq: HeapWaitQueue,
    /// Woken when B can recv (data in `a_to_b`).
    b_recv_wq: HeapWaitQueue,
    /// Number of active A-side endpoint handles.
    a_endpoints: AtomicUsize,
    /// Number of active B-side endpoint handles.
    b_endpoints: AtomicUsize,
}

impl ChannelInner {
    /// Returns the send queue and peer's recv wait queue for the given side.
    fn send_queue(
        &self,
        is_a: bool,
    ) -> (
        &SpinLock<alloc::collections::VecDeque<Vec<u8>>>,
        &HeapWaitQueue,
        &HeapWaitQueue,
    ) {
        if is_a {
            (&self.a_to_b, &self.a_send_wq, &self.b_recv_wq)
        } else {
            (&self.b_to_a, &self.b_send_wq, &self.a_recv_wq)
        }
    }

    /// Returns the recv queue and peer's send wait queue for the given side.
    fn recv_queue(
        &self,
        is_a: bool,
    ) -> (
        &SpinLock<alloc::collections::VecDeque<Vec<u8>>>,
        &HeapWaitQueue,
        &HeapWaitQueue,
    ) {
        if is_a {
            (&self.b_to_a, &self.a_recv_wq, &self.b_send_wq)
        } else {
            (&self.a_to_b, &self.b_recv_wq, &self.a_send_wq)
        }
    }

    /// Returns the peer's endpoint counter for the given side.
    fn peer_counter(&self, is_a: bool) -> &AtomicUsize {
        if is_a {
            &self.b_endpoints
        } else {
            &self.a_endpoints
        }
    }
}

/// One endpoint of a bidirectional channel.
pub struct ChannelEndpoint {
    /// Shared channel state.
    inner: Arc<ChannelInner>,
    /// `true` for the A side, `false` for the B side.
    is_a: bool,
}

impl Drop for ChannelEndpoint {
    fn drop(&mut self) {
        let counter = if self.is_a {
            &self.inner.a_endpoints
        } else {
            &self.inner.b_endpoints
        };
        counter.fetch_sub(1, Ordering::Release);

        // Wake all peer waiters so they see the endpoint is dead.
        if self.is_a {
            self.inner.b_recv_wq.wake_all();
            self.inner.b_send_wq.wake_all();
        } else {
            self.inner.a_recv_wq.wake_all();
            self.inner.a_send_wq.wake_all();
        }
    }
}

impl Inode for ChannelEndpoint {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        let (queue, _, _) = self.inner.recv_queue(self.is_a);
        queue.lock().len()
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    /// Write = send a message to the peer.
    ///
    /// The entire buffer is enqueued as one discrete message.
    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if buf.len() > MAX_MESSAGE_SIZE {
                return Err(FsError::InvalidArgument);
            }

            let (send_q, self_send_wq, peer_recv_wq) = self.inner.send_queue(self.is_a);
            let peer_counter = self.inner.peer_counter(self.is_a);

            loop {
                // Register-before-check to prevent lost wakeups.
                core::future::poll_fn(|cx| {
                    self_send_wq.register_waker(cx.waker());
                    let queue = send_q.lock();
                    if peer_counter.load(Ordering::Acquire) == 0
                        || queue.len() < MAX_BUFFERED_MESSAGES
                    {
                        core::task::Poll::Ready(())
                    } else {
                        core::task::Poll::Pending
                    }
                })
                .await;

                // Check if peer is dead.
                if peer_counter.load(Ordering::Acquire) == 0 {
                    return Err(FsError::BrokenPipe);
                }

                let mut queue = send_q.lock();
                if queue.len() < MAX_BUFFERED_MESSAGES {
                    let msg = buf.to_vec();
                    let len = msg.len();
                    queue.push_back(msg);
                    drop(queue);
                    // Wake peer's recv side.
                    peer_recv_wq.wake_one();
                    return Ok(len);
                }
                // Spurious wake — retry.
            }
        })
    }

    /// Read = receive a message from the peer.
    ///
    /// Dequeues one message and copies it into `buf`. Returns the message
    /// length (truncated if `buf` is smaller than the message).
    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let (recv_q, self_recv_wq, peer_send_wq) = self.inner.recv_queue(self.is_a);
            let peer_counter = self.inner.peer_counter(self.is_a);

            loop {
                // Register-before-check to prevent lost wakeups.
                core::future::poll_fn(|cx| {
                    self_recv_wq.register_waker(cx.waker());
                    let queue = recv_q.lock();
                    if !queue.is_empty() || peer_counter.load(Ordering::Acquire) == 0 {
                        core::task::Poll::Ready(())
                    } else {
                        core::task::Poll::Pending
                    }
                })
                .await;

                let mut queue = recv_q.lock();
                if let Some(msg) = queue.pop_front() {
                    let copy_len = msg.len().min(buf.len());
                    buf[..copy_len].copy_from_slice(&msg[..copy_len]);
                    drop(queue);
                    // Wake peer's send side (space freed).
                    peer_send_wq.wake_one();
                    return Ok(msg.len());
                }
                // Queue empty — check if all peers are gone.
                if peer_counter.load(Ordering::Acquire) == 0 {
                    return Ok(0); // EOF
                }
                // Spurious wake — retry.
            }
        })
    }

    fn lookup<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn poll_readiness(&self, waker: Option<&core::task::Waker>) -> u16 {
        use hadron_syscall::{POLLHUP, POLLIN, POLLOUT};

        let (recv_q, self_recv_wq, _) = self.inner.recv_queue(self.is_a);
        let (send_q, self_send_wq, _) = self.inner.send_queue(self.is_a);
        let peer_counter = self.inner.peer_counter(self.is_a);

        if let Some(w) = waker {
            self_recv_wq.register_waker(w);
            self_send_wq.register_waker(w);
        }

        let mut events = 0u16;

        let recv_queue = recv_q.lock();
        if !recv_queue.is_empty() {
            events |= POLLIN;
        }
        drop(recv_queue);

        let send_queue = send_q.lock();
        if send_queue.len() < MAX_BUFFERED_MESSAGES {
            events |= POLLOUT;
        }
        drop(send_queue);

        if peer_counter.load(Ordering::Acquire) == 0 {
            events |= POLLHUP;
            events |= POLLIN; // EOF is also readable.
        }

        events
    }
}
