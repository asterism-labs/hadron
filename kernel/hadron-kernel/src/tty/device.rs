//! DevTty — `/dev/ttyN` inode backed by a [`Tty`](super::Tty).
//!
//! Implements the VFS [`Inode`] trait so that userspace can open, read, and
//! write virtual terminals through the standard file descriptor interface.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};

use super::Tty;

/// `/dev/ttyN` — reads block for keyboard input, writes go to console output.
pub struct DevTty {
    /// The backing TTY instance.
    tty: &'static Tty,
}

impl DevTty {
    /// Create a new DevTty backed by the given TTY.
    pub const fn new(tty: &'static Tty) -> Self {
        Self { tty }
    }
}

/// Create a boxed read future for the given TTY.
///
/// Used by [`DevConsole`](crate::fs::devfs::DevConsole) to delegate reads to
/// the active TTY without constructing a temporary `DevTty`.
pub fn tty_read_future<'a>(
    tty: &'static Tty,
    buf: &'a mut [u8],
) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
    Box::pin(TtyReadFuture {
        tty,
        buf,
        subscribed: false,
    })
}

/// Future for reading from a TTY.
///
/// Uses a two-phase poll strategy to avoid registering noop wakers:
/// - **First poll**: check for data only — no waker subscription. Self-wake
///   ensures the `.await` path re-polls immediately.
/// - **Subsequent polls**: subscribe the real waker, then recheck to avoid
///   the race between "no data available" and "waker registered".
struct TtyReadFuture<'a> {
    tty: &'static Tty,
    buf: &'a mut [u8],
    /// Whether we have already subscribed a waker with the TTY's input waker.
    subscribed: bool,
}

impl Future for TtyReadFuture<'_> {
    type Output = Result<usize, FsError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // Always poll hardware and check for data first.
        this.tty.poll_hardware();
        if let Some(n) = this.tty.try_read(this.buf) {
            return Poll::Ready(Ok(n));
        }

        if !this.subscribed {
            // First poll: skip subscribe to avoid registering a noop waker
            // from try_poll_immediate. Self-wake ensures re-poll with real waker.
            this.subscribed = true;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }

        // Subsequent polls: register waker for IRQ notification.
        this.tty.subscribe(cx.waker());

        // Re-check after registration (catches IRQs between check and subscribe).
        this.tty.poll_hardware();
        if let Some(n) = this.tty.try_read(this.buf) {
            return Poll::Ready(Ok(n));
        }

        // Check for pending signals on the current (reading) process.
        let has_signal = crate::proc::try_current_process(|proc| proc.signals.has_pending());
        if has_signal == Some(true) {
            return Poll::Ready(Err(FsError::Interrupted));
        }

        Poll::Pending
    }
}

impl Inode for DevTty {
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
        Box::pin(TtyReadFuture {
            tty: self.tty,
            buf,
            subscribed: false,
        })
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
