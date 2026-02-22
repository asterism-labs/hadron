//! Device filesystem (`/dev`).
//!
//! Provides virtual device files:
//! - `/dev/null` -- reads return 0 bytes, writes are discarded
//! - `/dev/zero` -- reads fill buffer with zeros, writes are discarded
//! - `/dev/console` -- writes go to kernel console, reads block for keyboard input

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::ToString;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use super::{DirEntry, FileSystem, FsError, Inode, InodeType, Permissions};

/// The devfs filesystem.
pub struct DevFs {
    /// Root directory containing device entries.
    root: Arc<DevFsDir>,
}

impl Default for DevFs {
    fn default() -> Self {
        Self::new()
    }
}

impl DevFs {
    /// Creates a new devfs with standard device entries.
    #[must_use]
    pub fn new() -> Self {
        let mut entries: BTreeMap<&str, Arc<dyn Inode>> = BTreeMap::new();
        entries.insert("null", Arc::new(DevNull));
        entries.insert("zero", Arc::new(DevZero));
        entries.insert("console", Arc::new(DevConsole));

        Self {
            root: Arc::new(DevFsDir { entries }),
        }
    }
}

impl FileSystem for DevFs {
    fn name(&self) -> &'static str {
        "devfs"
    }

    fn root(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }
}

/// The devfs root directory.
struct DevFsDir {
    /// Fixed device entries.
    entries: BTreeMap<&'static str, Arc<dyn Inode>>,
}

impl Inode for DevFsDir {
    fn inode_type(&self) -> InodeType {
        InodeType::Directory
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::IsADirectory) })
    }

    fn lookup<'a>(
        &'a self,
        name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async move { self.entries.get(name).cloned().ok_or(FsError::NotFound) })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async move {
            Ok(self
                .entries
                .iter()
                .map(|(name, inode)| DirEntry {
                    name: (*name).to_string(),
                    inode_type: inode.inode_type(),
                })
                .collect())
        })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }
}

// ── /dev/null ──────────────────────────────────────────────────────────

/// `/dev/null` -- reads return EOF, writes are silently discarded.
struct DevNull;

impl Inode for DevNull {
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
        Box::pin(async { Ok(0) })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move { Ok(buf.len()) })
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
}

// ── /dev/zero ──────────────────────────────────────────────────────────

/// `/dev/zero` -- reads fill the buffer with zeros, writes are discarded.
struct DevZero;

impl Inode for DevZero {
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
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            buf.fill(0);
            Ok(buf.len())
        })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move { Ok(buf.len()) })
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
}

// ── /dev/console ───────────────────────────────────────────────────────

/// `/dev/console` -- writes go to kernel console output, reads block for keyboard input.
///
/// Reads use IRQ-driven notification: a keyboard IRQ wakes the reader future
/// which then polls the i8042 PS/2 controller for scancodes. This allows the
/// async executor to run other tasks while waiting for input.
pub struct DevConsole;

/// Diagnostic counters for `ConsoleReadFuture` poll phases.
#[cfg(hadron_lock_debug)]
pub(crate) mod console_read_diag {
    use core::sync::atomic::{AtomicU64, Ordering};

    /// Total number of `ConsoleReadFuture::poll` invocations.
    pub static POLL_COUNT: AtomicU64 = AtomicU64::new(0);
    /// Number of first-poll self-wakes (noop-waker avoidance phase).
    pub static POLL_FIRST: AtomicU64 = AtomicU64::new(0);
    /// Number of subscribe-phase polls (waker registered, waiting for IRQ).
    pub static POLL_SUBSCRIBE: AtomicU64 = AtomicU64::new(0);
    /// Number of polls that returned data successfully.
    pub static POLL_DATA_READY: AtomicU64 = AtomicU64::new(0);

    #[inline]
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// Future for reading from `/dev/console`.
///
/// Uses a two-phase poll strategy to avoid registering noop wakers:
/// - **First poll**: check for data only — no waker subscription. Self-wake
///   ensures the `.await` path re-polls immediately.
/// - **Subsequent polls**: subscribe the real waker, then check-subscribe-recheck
///   to avoid the race between "no data available" and "waker registered".
///
/// This prevents `try_poll_immediate` (which uses a noop waker) from polluting
/// the `INPUT_READY` wait queue with a waker that silently drops events.
struct ConsoleReadFuture<'a> {
    buf: &'a mut [u8],
    /// Whether we have already subscribed a waker with the input wait queue.
    subscribed: bool,
}

impl Future for ConsoleReadFuture<'_> {
    type Output = Result<usize, FsError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        #[cfg(hadron_lock_debug)]
        console_read_diag::inc(&console_read_diag::POLL_COUNT);

        // Always poll hardware and check for data first.
        super::console_input::poll_keyboard_hardware();
        let n = super::console_input::try_read(this.buf);
        if n > 0 {
            #[cfg(hadron_lock_debug)]
            console_read_diag::inc(&console_read_diag::POLL_DATA_READY);
            return Poll::Ready(Ok(n));
        }

        if !this.subscribed {
            // First poll: skip subscribe to avoid registering a noop waker
            // from try_poll_immediate. Self-wake ensures the .await path
            // re-polls us immediately so we can subscribe with the real waker.
            this.subscribed = true;
            #[cfg(hadron_lock_debug)]
            console_read_diag::inc(&console_read_diag::POLL_FIRST);
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }

        // Subsequent polls: register waker for IRQ notification.
        #[cfg(hadron_lock_debug)]
        console_read_diag::inc(&console_read_diag::POLL_SUBSCRIBE);
        super::console_input::subscribe(cx.waker());

        // Re-check after registration (catches IRQs between check and subscribe).
        super::console_input::poll_keyboard_hardware();
        let n = super::console_input::try_read(this.buf);
        if n > 0 {
            #[cfg(hadron_lock_debug)]
            console_read_diag::inc(&console_read_diag::POLL_DATA_READY);
            return Poll::Ready(Ok(n));
        }

        // Check for pending signals.
        if let Some(fg_pid) = crate::proc::foreground_pid() {
            if let Some(proc) = crate::proc::lookup_process(fg_pid) {
                if proc.signals.has_pending() {
                    return Poll::Ready(Err(FsError::Interrupted));
                }
            }
        }

        Poll::Pending
    }
}

impl Inode for DevConsole {
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
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(ConsoleReadFuture { buf, subscribed: false })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if let Ok(s) = core::str::from_utf8(buf) {
                crate::kprint!("{}", s);
            } else {
                for &byte in buf {
                    crate::kprint!("{}", byte as char);
                }
            }
            Ok(buf.len())
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
}
