//! AF_UNIX domain socket implementation.
//!
//! Provides bidirectional byte-stream sockets with `SCM_RIGHTS` fd-passing,
//! sufficient to run a standard Wayland compositor.
//!
//! # State machine
//!
//! ```text
//! Unbound → Bound(path) → Listening → (accept) Connected
//!                                  ↗
//!                        connect() ─ Connected
//! ```
//!
//! # Blocking
//!
//! - `accept()` — uses `TrapReason::Accept` to block in the kernel async loop.
//! - `read()`  — uses `TrapReason::Io` (TRAP_IO) as normal stream I/O.
//! - `write()` — uses `TrapReason::Io` for back-pressure on a full buffer.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, Ordering};

use hadron_core::sync::{HeapWaitQueue, SpinLock};

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};

// ── Constants ────────────────────────────────────────────────────────────────

/// Capacity of each stream ring buffer in bytes.
const SOCK_BUF_CAPACITY: usize = 65_536; // 64 KiB

/// Maximum number of pending file-descriptor transfers per buffer.
const MAX_PENDING_FDS: usize = 64;

/// Maximum number of pending connections in a listen backlog.
const MAX_BACKLOG: usize = 128;

// ── Global path registry ─────────────────────────────────────────────────────

/// Maps bound socket paths to their listening `UnixSocket` inodes.
static UNIX_REGISTRY: SpinLock<alloc::collections::BTreeMap<String, Arc<UnixSocket>>> =
    SpinLock::leveled("unix_registry", 3, alloc::collections::BTreeMap::new());

/// Bind a path in the global registry.
///
/// Returns `Err(FsError::AlreadyExists)` if the path is already bound.
pub(crate) fn registry_bind(path: &str, socket: Arc<UnixSocket>) -> Result<(), FsError> {
    let mut reg = UNIX_REGISTRY.lock();
    if reg.contains_key(path) {
        return Err(FsError::AlreadyExists);
    }
    reg.insert(path.to_string(), socket);
    Ok(())
}

/// Look up a bound (listening) socket by path.
pub(crate) fn registry_lookup(path: &str) -> Option<Arc<UnixSocket>> {
    UNIX_REGISTRY.lock().get(path).cloned()
}

/// Remove a path from the global registry (called on socket close).
pub(crate) fn registry_unbind(path: &str) {
    UNIX_REGISTRY.lock().remove(path);
}

// ── StreamBuffer ─────────────────────────────────────────────────────────────

/// A bounded ring buffer for byte data plus a queue of pending inodes for
/// `SCM_RIGHTS` fd-passing.
struct StreamBuffer {
    data: VecDeque<u8>,
    /// Queued file-descriptor inodes received via `SCM_RIGHTS`.
    pending_fds: VecDeque<Arc<dyn Inode>>,
    capacity: usize,
}

impl StreamBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: VecDeque::with_capacity(capacity.min(4096)),
            pending_fds: VecDeque::new(),
            capacity,
        }
    }

    /// Bytes available for reading.
    fn available(&self) -> usize {
        self.data.len()
    }

    /// Free bytes available for writing.
    fn free(&self) -> usize {
        self.capacity.saturating_sub(self.data.len())
    }

    /// Read up to `buf.len()` bytes. Returns number of bytes read.
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = self.data.len().min(buf.len());
        for byte in buf.iter_mut().take(n) {
            *byte = self.data.pop_front().unwrap_or(0);
        }
        n
    }

    /// Write up to `buf.len()` bytes (respecting capacity). Returns bytes written.
    fn write(&mut self, buf: &[u8]) -> usize {
        let space = self.free();
        let n = buf.len().min(space);
        self.data.extend(&buf[..n]);
        n
    }

    /// Enqueue a file descriptor inode for SCM_RIGHTS transfer.
    fn enqueue_fd(&mut self, inode: Arc<dyn Inode>) {
        if self.pending_fds.len() < MAX_PENDING_FDS {
            self.pending_fds.push_back(inode);
        }
    }

    /// Dequeue the next pending fd inode, if any.
    fn dequeue_fd(&mut self) -> Option<Arc<dyn Inode>> {
        self.pending_fds.pop_front()
    }
}

// ── ConnectedPair ─────────────────────────────────────────────────────────────

/// Shared state for a connected socket pair.
///
/// Side A is the client (the socket that called `connect()`).
/// Side B is the server (the socket returned by `accept()`).
pub(crate) struct ConnectedPair {
    /// Data written by side A, read by side B.
    a_to_b: SpinLock<StreamBuffer>,
    /// Data written by side B, read by side A.
    b_to_a: SpinLock<StreamBuffer>,
    /// Woken when side A can make progress (B wrote data or B read some A-data).
    wq_a: HeapWaitQueue,
    /// Woken when side B can make progress (A wrote data or A read some B-data).
    wq_b: HeapWaitQueue,
    /// Set when side A has shut down its write half or closed.
    pub closed_a: AtomicBool,
    /// Set when side B has shut down its write half or closed.
    pub closed_b: AtomicBool,
}

impl ConnectedPair {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            a_to_b: SpinLock::leveled("unix_a2b", 3, StreamBuffer::new(SOCK_BUF_CAPACITY)),
            b_to_a: SpinLock::leveled("unix_b2a", 3, StreamBuffer::new(SOCK_BUF_CAPACITY)),
            wq_a: HeapWaitQueue::new(),
            wq_b: HeapWaitQueue::new(),
            closed_a: AtomicBool::new(false),
            closed_b: AtomicBool::new(false),
        })
    }
}

// ── SocketState ───────────────────────────────────────────────────────────────

enum SocketState {
    /// Fresh socket, not yet bound.
    Unbound,
    /// Bound to `path` but not yet listening.
    Bound(String),
    /// Listening on `path` with a pending connection queue.
    Listening {
        path: String,
        backlog: usize,
        /// Server-side sockets ready to be returned by `accept()`.
        queue: VecDeque<Arc<UnixSocket>>,
    },
    /// Connected through `pair`. `is_a = true` for the connect() side.
    Connected {
        pair: Arc<ConnectedPair>,
        is_a: bool,
    },
    /// Closed.
    Closed,
}

// ── UnixSocket ────────────────────────────────────────────────────────────────

/// AF_UNIX stream socket implementing [`Inode`].
pub struct UnixSocket {
    /// Weak self-reference, set immediately after `Arc::new`.
    ///
    /// Required by `unix_listen` and `unix_connect` (called via the `Inode`
    /// trait) which need `Arc<UnixSocket>` to register with the global path
    /// registry or to create a `ConnectedPair`.
    self_weak: SpinLock<Option<alloc::sync::Weak<UnixSocket>>>,
    inner: SpinLock<SocketState>,
    /// Woken on state change: new connection available (listening) or
    /// data/space available (connected).
    wq: HeapWaitQueue,
}

impl UnixSocket {
    /// Create a new, unbound socket.
    pub fn new() -> Arc<Self> {
        let arc = Arc::new(Self {
            self_weak: SpinLock::leveled("unix_self_weak", 3, None),
            inner: SpinLock::leveled("unix_socket", 3, SocketState::Unbound),
            wq: HeapWaitQueue::new(),
        });
        *arc.self_weak.lock() = Some(Arc::downgrade(&arc));
        arc
    }

    /// Bind to `path`.
    ///
    /// Returns `Err(FsError::AlreadyExists)` if the path is already bound or
    /// `Err(FsError::InvalidArgument)` if the socket is not in the Unbound state.
    pub fn bind(self: &Arc<Self>, path: &str) -> Result<(), FsError> {
        let mut guard = self.inner.lock();
        match &*guard {
            SocketState::Unbound => {}
            _ => return Err(FsError::InvalidArgument),
        }
        *guard = SocketState::Bound(path.to_string());
        drop(guard);
        // Not yet registered in the global registry — that happens on listen().
        Ok(())
    }

    /// Mark the socket as listening.
    ///
    /// Registers the path in the global registry so that `connect()` can find it.
    pub fn listen(self: &Arc<Self>, backlog: usize) -> Result<(), FsError> {
        let path = {
            let guard = self.inner.lock();
            match &*guard {
                SocketState::Bound(p) => p.clone(),
                _ => return Err(FsError::InvalidArgument),
            }
        };

        let backlog = backlog.min(MAX_BACKLOG);
        {
            let mut guard = self.inner.lock();
            *guard = SocketState::Listening {
                path: path.clone(),
                backlog,
                queue: VecDeque::new(),
            };
        }

        registry_bind(&path, Arc::clone(self))?;
        Ok(())
    }

    /// Connect to the socket bound at `path`.
    ///
    /// Creates a `ConnectedPair` and pushes the server-side socket into the
    /// listener's accept queue, waking any blocked `accept()`.
    pub fn connect(self: &Arc<Self>, path: &str) -> Result<(), FsError> {
        // Must be Unbound to connect.
        {
            let guard = self.inner.lock();
            if !matches!(&*guard, SocketState::Unbound) {
                return Err(FsError::InvalidArgument);
            }
        }

        let listener = registry_lookup(path).ok_or(FsError::NotFound)?;

        // Create the connected pair.
        let pair = ConnectedPair::new();

        // Create the server-side socket (already Connected, is_a = false).
        // self_weak is None — this socket is Connected and never needs bind/listen.
        let server_socket = Arc::new(UnixSocket {
            self_weak: SpinLock::leveled("unix_self_weak", 3, None),
            inner: SpinLock::leveled(
                "unix_socket",
                3,
                SocketState::Connected {
                    pair: Arc::clone(&pair),
                    is_a: false,
                },
            ),
            wq: HeapWaitQueue::new(),
        });

        // Push the server socket into the listener's accept queue.
        {
            let mut guard = listener.inner.lock();
            match &mut *guard {
                SocketState::Listening { queue, backlog, .. } => {
                    if queue.len() >= *backlog {
                        return Err(FsError::IoError); // ECONNREFUSED analogue
                    }
                    queue.push_back(server_socket);
                }
                _ => return Err(FsError::NotFound), // not listening
            }
        }
        listener.wq.wake_one(); // wake blocked accept()

        // Set the client side to Connected (is_a = true).
        {
            let mut guard = self.inner.lock();
            *guard = SocketState::Connected { pair, is_a: true };
        }

        Ok(())
    }

    /// Shut down the write half (or both halves) of the socket.
    pub fn shutdown(&self, shut_write: bool) {
        let guard = self.inner.lock();
        if let SocketState::Connected { pair, is_a } = &*guard {
            if *is_a && shut_write {
                pair.closed_a.store(true, Ordering::Release);
                pair.wq_b.wake_all(); // wake peer so it sees EOF
            } else if !is_a && shut_write {
                pair.closed_b.store(true, Ordering::Release);
                pair.wq_a.wake_all();
            }
        }
    }

    /// Enqueue a file-descriptor inode into the send buffer.
    ///
    /// Called by `sendmsg` for each `SCM_RIGHTS` fd. The fd is queued in the
    /// TX buffer so the receiver can dequeue it with `dequeue_recv_fd`.
    fn enqueue_fd_for_send(&self, inode: Arc<dyn Inode>) {
        let guard = self.inner.lock();
        if let SocketState::Connected { pair, is_a } = &*guard {
            if *is_a {
                pair.a_to_b.lock().enqueue_fd(inode);
            } else {
                pair.b_to_a.lock().enqueue_fd(inode);
            }
        }
    }

    /// Dequeue the next received file-descriptor inode.
    ///
    /// Called by `recvmsg` after reading data to retrieve accompanying fds.
    fn dequeue_recv_fd_inner(&self) -> Option<Arc<dyn Inode>> {
        let guard = self.inner.lock();
        if let SocketState::Connected { pair, is_a } = &*guard {
            if *is_a {
                pair.b_to_a.lock().dequeue_fd()
            } else {
                pair.a_to_b.lock().dequeue_fd()
            }
        } else {
            None
        }
    }
}

impl Default for UnixSocket {
    fn default() -> Self {
        Self {
            self_weak: SpinLock::leveled("unix_self_weak", 3, None),
            inner: SpinLock::leveled("unix_socket", 3, SocketState::Unbound),
            wq: HeapWaitQueue::new(),
        }
    }
}

impl Drop for UnixSocket {
    fn drop(&mut self) {
        // Unregister from global path registry and signal peer.
        // NOTE: self.inner (unix_socket, level 3) must not be acquired while
        // any level-4+ lock (e.g. fd_table) is held. Callers that close an fd
        // containing a UnixSocket must use close_take() to defer the drop until
        // after releasing fd_table. See docs/src/reference/known-issues.md.
        crate::ktrace_subsys!(net, "UnixSocket::drop: acquiring inner lock");
        let guard = self.inner.lock();
        crate::ktrace_subsys!(net, "UnixSocket::drop: inner locked, processing state");
        match &*guard {
            SocketState::Bound(p) | SocketState::Listening { path: p, .. } => {
                crate::ktrace_subsys!(net, "UnixSocket::drop: unbinding path {}", p);
                registry_unbind(p);
            }
            SocketState::Connected { pair, is_a } => {
                crate::ktrace_subsys!(net, "UnixSocket::drop: signalling peer (is_a={})", is_a);
                if *is_a {
                    pair.closed_a.store(true, Ordering::Release);
                    pair.wq_b.wake_all(); // peer sees EOF
                } else {
                    pair.closed_b.store(true, Ordering::Release);
                    pair.wq_a.wake_all();
                }
            }
            _ => {}
        }
    }
}

// ── Inode implementation ──────────────────────────────────────────────────────

impl Inode for UnixSocket {
    fn inode_type(&self) -> InodeType {
        InodeType::Socket
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions {
            read: true,
            write: true,
            execute: false,
        }
    }

    /// Stream read: blocks until data is available or the peer has closed.
    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            // Extract connected pair (avoids holding lock during .await).
            let (pair, is_a) = {
                let guard = self.inner.lock();
                match &*guard {
                    SocketState::Connected { pair, is_a } => (Arc::clone(pair), *is_a),
                    SocketState::Closed => return Ok(0),
                    _ => return Err(FsError::NotSupported),
                }
            };

            loop {
                // Register-before-check pattern to avoid lost wakeups.
                core::future::poll_fn(|cx| {
                    if is_a {
                        pair.wq_a.register_waker(cx.waker());
                        let rx = pair.b_to_a.lock();
                        if rx.available() > 0 || pair.closed_b.load(Ordering::Acquire) {
                            core::task::Poll::Ready(())
                        } else {
                            core::task::Poll::Pending
                        }
                    } else {
                        pair.wq_b.register_waker(cx.waker());
                        let rx = pair.a_to_b.lock();
                        if rx.available() > 0 || pair.closed_a.load(Ordering::Acquire) {
                            core::task::Poll::Ready(())
                        } else {
                            core::task::Poll::Pending
                        }
                    }
                })
                .await;

                if is_a {
                    let mut rx = pair.b_to_a.lock();
                    if rx.available() > 0 {
                        let n = rx.read(buf);
                        drop(rx);
                        pair.wq_b.wake_all(); // B may write more now
                        return Ok(n);
                    }
                    if pair.closed_b.load(Ordering::Acquire) {
                        return Ok(0); // EOF
                    }
                } else {
                    let mut rx = pair.a_to_b.lock();
                    if rx.available() > 0 {
                        let n = rx.read(buf);
                        drop(rx);
                        pair.wq_a.wake_all(); // A may write more now
                        return Ok(n);
                    }
                    if pair.closed_a.load(Ordering::Acquire) {
                        return Ok(0); // EOF
                    }
                }
                // Spurious wakeup — retry.
            }
        })
    }

    /// Stream write: blocks until space is available or the peer has closed.
    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            let (pair, is_a) = {
                let guard = self.inner.lock();
                match &*guard {
                    SocketState::Connected { pair, is_a } => (Arc::clone(pair), *is_a),
                    SocketState::Closed => return Err(FsError::BrokenPipe),
                    _ => return Err(FsError::NotSupported),
                }
            };

            // Check if peer has closed.
            let peer_closed = if is_a {
                pair.closed_b.load(Ordering::Acquire)
            } else {
                pair.closed_a.load(Ordering::Acquire)
            };
            if peer_closed {
                return Err(FsError::BrokenPipe);
            }

            loop {
                // Register-before-check.
                core::future::poll_fn(|cx| {
                    if is_a {
                        pair.wq_a.register_waker(cx.waker());
                        let tx = pair.a_to_b.lock();
                        if tx.free() > 0 || pair.closed_b.load(Ordering::Acquire) {
                            core::task::Poll::Ready(())
                        } else {
                            core::task::Poll::Pending
                        }
                    } else {
                        pair.wq_b.register_waker(cx.waker());
                        let tx = pair.b_to_a.lock();
                        if tx.free() > 0 || pair.closed_a.load(Ordering::Acquire) {
                            core::task::Poll::Ready(())
                        } else {
                            core::task::Poll::Pending
                        }
                    }
                })
                .await;

                if is_a {
                    if pair.closed_b.load(Ordering::Acquire) {
                        return Err(FsError::BrokenPipe);
                    }
                    let n = pair.a_to_b.lock().write(buf);
                    if n > 0 {
                        pair.wq_b.wake_all(); // B can now read
                        return Ok(n);
                    }
                } else {
                    if pair.closed_a.load(Ordering::Acquire) {
                        return Err(FsError::BrokenPipe);
                    }
                    let n = pair.b_to_a.lock().write(buf);
                    if n > 0 {
                        pair.wq_a.wake_all(); // A can now read
                        return Ok(n);
                    }
                }
                // Spurious wakeup — retry.
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
        let guard = self.inner.lock();
        match &*guard {
            SocketState::Listening { queue, .. } => {
                if let Some(w) = waker {
                    self.wq.register_waker(w);
                }
                if queue.is_empty() { 0 } else { POLLIN }
            }
            SocketState::Connected { pair, is_a } => {
                if let Some(w) = waker {
                    if *is_a {
                        pair.wq_a.register_waker(w);
                    } else {
                        pair.wq_b.register_waker(w);
                    }
                }
                let (rx_has_data, tx_has_space, peer_closed) = if *is_a {
                    let rx = pair.b_to_a.lock();
                    (
                        rx.available() > 0,
                        pair.a_to_b.lock().free() > 0,
                        pair.closed_b.load(Ordering::Acquire),
                    )
                } else {
                    let rx = pair.a_to_b.lock();
                    (
                        rx.available() > 0,
                        pair.b_to_a.lock().free() > 0,
                        pair.closed_a.load(Ordering::Acquire),
                    )
                };
                let mut events = 0u16;
                if rx_has_data || peer_closed {
                    events |= POLLIN;
                }
                if tx_has_space && !peer_closed {
                    events |= POLLOUT;
                }
                if peer_closed {
                    events |= POLLHUP;
                }
                events
            }
            SocketState::Closed => POLLHUP,
            _ => 0,
        }
    }

    /// Accept an incoming connection. Blocks until one arrives.
    fn accept_connection(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + '_>> {
        Box::pin(async move {
            loop {
                // Register-before-check.
                core::future::poll_fn(|cx| {
                    self.wq.register_waker(cx.waker());
                    let guard = self.inner.lock();
                    match &*guard {
                        SocketState::Listening { queue, .. } if !queue.is_empty() => {
                            core::task::Poll::Ready(())
                        }
                        SocketState::Listening { .. } => core::task::Poll::Pending,
                        _ => core::task::Poll::Ready(()), // closed or wrong state
                    }
                })
                .await;

                let mut guard = self.inner.lock();
                match &mut *guard {
                    SocketState::Listening { queue, .. } => {
                        if let Some(socket) = queue.pop_front() {
                            return Ok(socket as Arc<dyn Inode>);
                        }
                        // Spurious wakeup — retry.
                    }
                    _ => return Err(FsError::InvalidArgument),
                }
            }
        })
    }

    fn dequeue_recv_fd(&self) -> Option<Arc<dyn Inode>> {
        self.dequeue_recv_fd_inner()
    }

    fn enqueue_send_fd(&self, inode: Arc<dyn Inode>) {
        self.enqueue_fd_for_send(inode);
    }

    fn unix_bind(&self, path: &str) -> Result<(), crate::fs::FsError> {
        let mut guard = self.inner.lock();
        match &*guard {
            SocketState::Unbound => {}
            _ => return Err(crate::fs::FsError::InvalidArgument),
        }
        *guard = SocketState::Bound(path.to_string());
        Ok(())
    }

    fn unix_listen(&self, backlog: usize) -> Result<(), crate::fs::FsError> {
        let arc = self
            .self_weak
            .lock()
            .as_ref()
            .and_then(alloc::sync::Weak::upgrade)
            .ok_or(crate::fs::FsError::InvalidArgument)?;
        arc.listen(backlog)
    }

    fn unix_connect(&self, path: &str) -> Result<(), crate::fs::FsError> {
        let arc = self
            .self_weak
            .lock()
            .as_ref()
            .and_then(alloc::sync::Weak::upgrade)
            .ok_or(crate::fs::FsError::InvalidArgument)?;
        arc.connect(path)
    }

    fn unix_shutdown(&self, how: u8) {
        // SHUT_RD(0) shuts down reads only — we model that as a no-op for now;
        // SHUT_WR(1) or SHUT_RDWR(2) close the write half.
        let shut_write = how != 0;
        self.shutdown(shut_write);
    }
}
