//! Byte-oriented pipe for IPC.
//!
//! A pipe has a reader half and a writer half, both implementing [`Inode`].
//! Data written to the writer is buffered in a circular buffer and can be
//! read from the reader. When all writers are dropped, the reader gets EOF.
//! When all readers are dropped, the writer gets `-EPIPE`.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use hadron_core::sync::atomic::{AtomicUsize, Ordering};

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};
use crate::sync::{HeapWaitQueue, SpinLock};

/// Default pipe buffer size: 64 KiB.
const PIPE_BUF_SIZE: usize = 64 * 1024;

/// Creates a new pipe, returning the reader and writer halves as `Arc<dyn Inode>`.
pub fn pipe() -> (Arc<dyn Inode>, Arc<dyn Inode>) {
    let inner = Arc::new(PipeInner {
        buffer: SpinLock::named("pipe_buffer", CircularBuffer::new(PIPE_BUF_SIZE)),
        read_wq: HeapWaitQueue::new(),
        write_wq: HeapWaitQueue::new(),
        readers: AtomicUsize::new(1),
        writers: AtomicUsize::new(1),
    });
    let reader = Arc::new(PipeReader(inner.clone()));
    let writer = Arc::new(PipeWriter(inner));
    (reader, writer)
}

/// Shared pipe state.
struct PipeInner {
    /// Circular buffer holding pipe data.
    buffer: SpinLock<CircularBuffer>,
    /// Woken when data becomes available for reading.
    read_wq: HeapWaitQueue,
    /// Woken when space becomes available for writing.
    write_wq: HeapWaitQueue,
    /// Number of active reader handles.
    readers: AtomicUsize,
    /// Number of active writer handles.
    writers: AtomicUsize,
}

use super::circular_buffer::CircularBuffer;

/// Reader half of a pipe.
pub struct PipeReader(Arc<PipeInner>);

/// Writer half of a pipe.
pub struct PipeWriter(Arc<PipeInner>);

impl Drop for PipeReader {
    fn drop(&mut self) {
        self.0.readers.fetch_sub(1, Ordering::Release);
        // Wake writers so they see readers == 0 and return EPIPE.
        self.0.write_wq.wake_all();
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.0.writers.fetch_sub(1, Ordering::Release);
        // Wake readers so they see writers == 0 and return EOF (0).
        self.0.read_wq.wake_all();
    }
}

impl Inode for PipeReader {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        self.0.buffer.lock().len()
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            loop {
                // Wait for data or EOF using register-before-check to
                // prevent lost wakeups on multi-CPU.
                core::future::poll_fn(|cx| {
                    self.0.read_wq.register_waker(cx.waker());
                    let buffer = self.0.buffer.lock();
                    if !buffer.is_empty() || self.0.writers.load(Ordering::Acquire) == 0 {
                        core::task::Poll::Ready(())
                    } else {
                        core::task::Poll::Pending
                    }
                })
                .await;

                // Perform actual read under lock.
                let mut buffer = self.0.buffer.lock();
                if !buffer.is_empty() {
                    let n = buffer.read(buf);
                    drop(buffer);
                    // Wake writers waiting for space.
                    self.0.write_wq.wake_one();
                    return Ok(n);
                }
                // Buffer empty — check if all writers are gone.
                if self.0.writers.load(Ordering::Acquire) == 0 {
                    return Ok(0); // EOF
                }
                // Spurious wake — retry.
            }
        })
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
        use hadron_syscall::{POLLHUP, POLLIN};

        if let Some(w) = waker {
            self.0.read_wq.register_waker(w);
        }
        let buffer = self.0.buffer.lock();
        let mut events = 0u16;
        if !buffer.is_empty() {
            events |= POLLIN;
        }
        if self.0.writers.load(Ordering::Acquire) == 0 {
            events |= POLLHUP;
            // EOF also counts as readable (read returns 0).
            events |= POLLIN;
        }
        events
    }
}

impl Inode for PipeWriter {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        self.0.buffer.lock().len()
    }

    fn permissions(&self) -> Permissions {
        Permissions {
            read: false,
            write: true,
            execute: false,
        }
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
        buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            loop {
                // Wait for space or broken pipe using register-before-check
                // to prevent lost wakeups on multi-CPU.
                core::future::poll_fn(|cx| {
                    self.0.write_wq.register_waker(cx.waker());
                    let buffer = self.0.buffer.lock();
                    if self.0.readers.load(Ordering::Acquire) == 0 || !buffer.is_full() {
                        core::task::Poll::Ready(())
                    } else {
                        core::task::Poll::Pending
                    }
                })
                .await;

                // Perform actual write under lock.
                let mut buffer = self.0.buffer.lock();
                // Check if readers are gone.
                if self.0.readers.load(Ordering::Acquire) == 0 {
                    return Err(FsError::BrokenPipe);
                }
                if !buffer.is_full() {
                    let n = buffer.write(buf);
                    drop(buffer);
                    // Wake readers waiting for data.
                    self.0.read_wq.wake_one();
                    return Ok(n);
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
        use hadron_syscall::{POLLERR, POLLOUT};

        if let Some(w) = waker {
            self.0.write_wq.register_waker(w);
        }
        let buffer = self.0.buffer.lock();
        let mut events = 0u16;
        if !buffer.is_full() {
            events |= POLLOUT;
        }
        if self.0.readers.load(Ordering::Acquire) == 0 {
            events |= POLLERR; // Broken pipe.
        }
        events
    }
}
