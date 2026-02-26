//! Inter-process communication primitives for Hadron OS.
//!
//! Provides channels for message-oriented IPC, pipes for byte-oriented IPC,
//! and service endpoints for dynamic client connections.
//!
//! This crate contains the pure IPC logic with no direct kernel dependencies.
//! Kernel-specific IPC (futex, shared memory) remains in `hadron-kernel`.

#![cfg_attr(not(test), no_std)]
#![warn(missing_docs)]

extern crate alloc;

pub mod channel;
pub mod circular_buffer;
pub mod pipe;
pub mod service;

#[cfg(test)]
mod tests {
    use super::circular_buffer::CircularBuffer;

    // -- CircularBuffer tests -------------------------------------------------

    #[test]
    fn cbuf_new_is_empty() {
        let buf = CircularBuffer::new(64);
        assert!(buf.is_empty());
        assert!(!buf.is_full());
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.capacity(), 64);
    }

    #[test]
    fn cbuf_write_then_read() {
        let mut buf = CircularBuffer::new(64);
        let written = buf.write(b"hello");
        assert_eq!(written, 5);
        assert_eq!(buf.len(), 5);

        let mut out = [0u8; 16];
        let read = buf.read(&mut out);
        assert_eq!(read, 5);
        assert_eq!(&out[..5], b"hello");
        assert!(buf.is_empty());
    }

    #[test]
    fn cbuf_wraparound() {
        let mut buf = CircularBuffer::new(8);
        // Fill 6 of 8 bytes.
        assert_eq!(buf.write(&[1, 2, 3, 4, 5, 6]), 6);
        // Read 4 bytes (advances read_pos to 4).
        let mut out = [0u8; 4];
        assert_eq!(buf.read(&mut out), 4);
        assert_eq!(out, [1, 2, 3, 4]);
        // Write 6 more bytes — wraps around.
        assert_eq!(buf.write(&[7, 8, 9, 10, 11, 12]), 6);
        assert_eq!(buf.len(), 8);
        assert!(buf.is_full());

        // Read all 8 bytes.
        let mut out = [0u8; 8];
        assert_eq!(buf.read(&mut out), 8);
        assert_eq!(out, [5, 6, 7, 8, 9, 10, 11, 12]);
    }

    #[test]
    fn cbuf_full_returns_zero_on_write() {
        let mut buf = CircularBuffer::new(4);
        assert_eq!(buf.write(&[1, 2, 3, 4]), 4);
        assert!(buf.is_full());
        assert_eq!(buf.write(&[5]), 0);
    }

    #[test]
    fn cbuf_empty_returns_zero_on_read() {
        let mut buf = CircularBuffer::new(4);
        let mut out = [0u8; 4];
        assert_eq!(buf.read(&mut out), 0);
    }

    #[test]
    fn cbuf_partial_write() {
        let mut buf = CircularBuffer::new(4);
        // Try to write 8 bytes into a 4-byte buffer.
        let written = buf.write(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(written, 4);
        assert!(buf.is_full());
    }
}
