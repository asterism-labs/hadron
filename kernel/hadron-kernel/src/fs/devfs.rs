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

/// Future for reading from `/dev/console`.
///
/// Uses check-register-recheck to avoid the race between
/// "no data available" and "waker registered":
/// 1. Poll keyboard + check buffer
/// 2. Register waker with IRQ wait queue
/// 3. Re-check buffer (catches IRQs between steps 1 and 2)
/// 4. Return Pending — next IRQ wake will re-poll this future
struct ConsoleReadFuture<'a> {
    buf: &'a mut [u8],
}

impl Future for ConsoleReadFuture<'_> {
    type Output = Result<usize, FsError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        // 1. Process any pending scancodes and try to read.
        super::console_input::poll_keyboard_hardware();
        let n = super::console_input::try_read(this.buf);
        if n > 0 {
            return Poll::Ready(Ok(n));
        }

        // 2. Register waker for keyboard IRQ notification.
        super::console_input::subscribe(cx.waker());

        // 3. Re-check after registration (catches IRQs between steps 1 and 2).
        super::console_input::poll_keyboard_hardware();
        let n = super::console_input::try_read(this.buf);
        if n > 0 {
            return Poll::Ready(Ok(n));
        }

        // 4. No data — yield to executor until keyboard IRQ fires.
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
        Box::pin(ConsoleReadFuture { buf })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if let Ok(s) = core::str::from_utf8(buf) {
                hadron_core::kprint!("{}", s);
            } else {
                for &byte in buf {
                    hadron_core::kprint!("{}", byte as char);
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
