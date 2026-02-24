//! Service endpoint for dynamic client connections.
//!
//! Models the `/dev/ptmx` `on_open()` pattern for services like the compositor.
//! Opening the connector inode creates a channel pair and queues the server end
//! for the listener. The `channel_accept` syscall dequeues pending connections
//! from the listener.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};
use crate::sync::{HeapWaitQueue, SpinLock};

/// Maximum pending connections before new opens are rejected.
const MAX_PENDING: usize = 16;

/// Shared state between the connector and listener.
struct ServiceInner {
    /// Pending server-side channel endpoints waiting to be accepted.
    pending: SpinLock<VecDeque<Arc<dyn Inode>>>,
    /// Woken when a new connection is queued.
    accept_wq: HeapWaitQueue,
}

/// Create a service endpoint pair.
///
/// Returns `(listener, connector)`:
/// - **Listener** is opened by the server (e.g. compositor) to accept connections.
/// - **Connector** is mounted at a well-known path (e.g. `/dev/compositor`)
///   for clients to open.
pub fn create_service() -> (Arc<ServiceListener>, Arc<ServiceConnector>) {
    let inner = Arc::new(ServiceInner {
        pending: SpinLock::named("service_pending", VecDeque::new()),
        accept_wq: HeapWaitQueue::new(),
    });
    let listener = Arc::new(ServiceListener {
        inner: inner.clone(),
    });
    let connector = Arc::new(ServiceConnector { inner });
    (listener, connector)
}

// ── ServiceListener ─────────────────────────────────────────────────

/// Server-side listener inode.
///
/// The compositor opens `/dev/compositor_listen` to obtain this inode. The
/// `channel_accept` syscall dequeues pending connections from here.
pub struct ServiceListener {
    inner: Arc<ServiceInner>,
}

impl ServiceListener {
    /// Dequeue the next pending connection, or `None` if empty.
    pub fn try_accept(&self) -> Option<Arc<dyn Inode>> {
        self.inner.pending.lock().pop_front()
    }
}

impl Inode for ServiceListener {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        self.inner.pending.lock().len()
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
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
        if let Some(w) = waker {
            self.inner.accept_wq.register_waker(w);
        }
        let pending = self.inner.pending.lock();
        if pending.is_empty() {
            0
        } else {
            hadron_syscall::POLLIN
        }
    }
}

// ── ServiceConnector ────────────────────────────────────────────────

/// Client-side connector inode.
///
/// Mounted at `/dev/compositor`. Each `open()` creates a new channel pair,
/// queues the server end for the listener, and returns the client end.
pub struct ServiceConnector {
    inner: Arc<ServiceInner>,
}

impl Inode for ServiceConnector {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_write()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
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

    /// Each open creates a channel pair: client end returned, server end queued.
    fn on_open(&self) -> Result<Option<Arc<dyn Inode>>, FsError> {
        let mut pending = self.inner.pending.lock();
        if pending.len() >= MAX_PENDING {
            return Err(FsError::NotSupported);
        }
        let (server_end, client_end) = crate::ipc::channel::channel();
        pending.push_back(server_end);
        drop(pending);
        self.inner.accept_wq.wake_one();
        Ok(Some(client_end))
    }
}
