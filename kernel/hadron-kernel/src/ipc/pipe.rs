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
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};
use crate::sync::{HeapWaitQueue, SpinLock};

/// Default pipe buffer size: 64 KiB.
const PIPE_BUF_SIZE: usize = 64 * 1024;

/// Creates a new pipe, returning the reader and writer halves as `Arc<dyn Inode>`.
pub fn pipe() -> (Arc<dyn Inode>, Arc<dyn Inode>) {
    let inner = Arc::new(PipeInner {
        buffer: SpinLock::new(CircularBuffer::new(PIPE_BUF_SIZE)),
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

/// Fixed-size circular buffer.
struct CircularBuffer {
    data: Box<[u8]>,
    read_pos: usize,
    write_pos: usize,
    count: usize,
}

impl CircularBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            data: alloc::vec![0u8; capacity].into_boxed_slice(),
            read_pos: 0,
            write_pos: 0,
            count: 0,
        }
    }

    fn capacity(&self) -> usize {
        self.data.len()
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn is_full(&self) -> bool {
        self.count == self.capacity()
    }

    /// Read up to `buf.len()` bytes from the buffer. Returns bytes read.
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = buf.len().min(self.count);
        let cap = self.capacity();
        for i in 0..to_read {
            buf[i] = self.data[(self.read_pos + i) % cap];
        }
        self.read_pos = (self.read_pos + to_read) % cap;
        self.count -= to_read;
        to_read
    }

    /// Write up to `buf.len()` bytes to the buffer. Returns bytes written.
    fn write(&mut self, buf: &[u8]) -> usize {
        let available = self.capacity() - self.count;
        let to_write = buf.len().min(available);
        let cap = self.capacity();
        for i in 0..to_write {
            self.data[(self.write_pos + i) % cap] = buf[i];
        }
        self.write_pos = (self.write_pos + to_write) % cap;
        self.count += to_write;
        to_write
    }
}

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
        self.0.buffer.lock().count
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
                {
                    let mut buffer = self.0.buffer.lock();
                    if !buffer.is_empty() {
                        let n = buffer.read(buf);
                        // Wake writers waiting for space.
                        self.0.write_wq.wake_one();
                        return Ok(n);
                    }
                    // Buffer empty — check if all writers are gone.
                    if self.0.writers.load(Ordering::Acquire) == 0 {
                        return Ok(0); // EOF
                    }
                }
                // Buffer empty and writers exist — wait.
                self.0.read_wq.wait().await;
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

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(
        &self,
        _name: &str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotADirectory)
    }
}

impl Inode for PipeWriter {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        self.0.buffer.lock().count
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
                {
                    let mut buffer = self.0.buffer.lock();
                    // Check if readers are gone.
                    if self.0.readers.load(Ordering::Acquire) == 0 {
                        return Err(FsError::IoError); // EPIPE
                    }
                    if !buffer.is_full() {
                        let n = buffer.write(buf);
                        // Wake readers waiting for data.
                        self.0.read_wq.wake_one();
                        return Ok(n);
                    }
                }
                // Buffer full and readers exist — wait.
                self.0.write_wq.wait().await;
            }
        })
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn readdir(&self) -> Result<Vec<DirEntry>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn create(
        &self,
        _name: &str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Result<Arc<dyn Inode>, FsError> {
        Err(FsError::NotADirectory)
    }

    fn unlink(&self, _name: &str) -> Result<(), FsError> {
        Err(FsError::NotADirectory)
    }
}
