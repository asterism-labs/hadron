//! Fixed-size circular byte buffer with bulk copy.
//!
//! Used by pipes and PTYs for buffered byte-stream I/O. Reads and writes
//! use at most two `copy_from_slice` calls (before and after wrap-around)
//! instead of per-byte modular indexing.

extern crate alloc;

use alloc::boxed::Box;

/// Fixed-size circular buffer backed by a heap-allocated byte slice.
pub(crate) struct CircularBuffer {
    data: Box<[u8]>,
    read_pos: usize,
    write_pos: usize,
    count: usize,
}

impl CircularBuffer {
    /// Create a new buffer with the given capacity (in bytes).
    pub fn new(capacity: usize) -> Self {
        Self {
            data: alloc::vec![0u8; capacity].into_boxed_slice(),
            read_pos: 0,
            write_pos: 0,
            count: 0,
        }
    }

    /// Total capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    /// Number of bytes currently in the buffer.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Returns `true` if the buffer contains no data.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns `true` if the buffer is completely full.
    pub fn is_full(&self) -> bool {
        self.count == self.data.len()
    }

    /// Read up to `buf.len()` bytes from the buffer. Returns the number of
    /// bytes actually read.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let to_read = buf.len().min(self.count);
        if to_read == 0 {
            return 0;
        }
        let cap = self.data.len();
        let first = (cap - self.read_pos).min(to_read);
        buf[..first].copy_from_slice(&self.data[self.read_pos..self.read_pos + first]);
        if first < to_read {
            buf[first..to_read].copy_from_slice(&self.data[..to_read - first]);
        }
        self.read_pos = (self.read_pos + to_read) % cap;
        self.count -= to_read;
        to_read
    }

    /// Write up to `buf.len()` bytes into the buffer. Returns the number of
    /// bytes actually written.
    pub fn write(&mut self, buf: &[u8]) -> usize {
        let available = self.data.len() - self.count;
        let to_write = buf.len().min(available);
        if to_write == 0 {
            return 0;
        }
        let cap = self.data.len();
        let first = (cap - self.write_pos).min(to_write);
        self.data[self.write_pos..self.write_pos + first].copy_from_slice(&buf[..first]);
        if first < to_write {
            self.data[..to_write - first].copy_from_slice(&buf[first..to_write]);
        }
        self.write_pos = (self.write_pos + to_write) % cap;
        self.count += to_write;
        to_write
    }
}
