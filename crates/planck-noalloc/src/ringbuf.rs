//! Circular/ring buffer implementation with fixed capacity.
//!
//! This module provides [`RingBuf`], a circular buffer (also known as a ring buffer)
//! that operates in a First-In-First-Out (FIFO) manner. It's particularly useful for
//! producer-consumer scenarios, buffering streams of data, or implementing queues
//! without heap allocation.
//!
//! # Overview
//!
//! A ring buffer is a fixed-size buffer that wraps around when it reaches the end,
//! forming a conceptual circle. Elements are added at the head and removed from the
//! tail, making it ideal for streaming data and FIFO queues.
//!
//! # Capacity
//!
//! The buffer has a compile-time fixed size `SIZE`, but the actual usable capacity
//! is `SIZE - 1`. This is a common implementation technique that simplifies the
//! empty/full detection logic.
//!
//! # Use Cases
//!
//! Ring buffers are excellent for:
//! - Buffering serial/UART data in embedded systems
//! - Implementing producer-consumer queues
//! - Audio/video frame buffers
//! - Network packet buffers
//! - Log message buffers
//! - Any scenario requiring a fixed-size FIFO queue
//!
//! # Performance
//!
//! - Push: O(1)
//! - Pop: O(1)
//! - Space efficient: uses exactly `SIZE * sizeof(T)` bytes
//! - Lock-free for single producer/single consumer scenarios
//!
//! # Examples
//!
//! ```
//! use planck_noalloc::ringbuf::RingBuf;
//!
//! // Create a ring buffer with size 8 (capacity 7)
//! let mut buf = RingBuf::<u8, 8>::new();
//!
//! // Push some data
//! buf.push(1);
//! buf.push(2);
//! buf.push(3);
//!
//! // Pop in FIFO order
//! assert_eq!(buf.pop(), Some(1));
//! assert_eq!(buf.pop(), Some(2));
//! assert_eq!(buf.len(), 1);
//!
//! // Can push more after popping
//! buf.push(4);
//! buf.push(5);
//!
//! assert_eq!(buf.len(), 3);
//! ```
//!
//! ## Error Handling
//!
//! ```
//! use planck_noalloc::ringbuf::RingBuf;
//!
//! let mut buf = RingBuf::<u8, 4>::new();
//!
//! // Fill to capacity (3 elements for size 4)
//! buf.push(1);
//! buf.push(2);
//! buf.push(3);
//!
//! // Trying to push beyond capacity returns an error
//! assert!(buf.try_push(4).is_err());
//! assert!(buf.is_full());
//!
//! // After popping, we can push again
//! buf.pop();
//! assert!(buf.try_push(4).is_ok());
//! ```

use core::mem::MaybeUninit;

/// A Circular / Ring buffer.
///
/// A fixed-capacity FIFO (First-In-First-Out) data structure that wraps around
/// when it reaches the end of its backing array. Elements are added at the head
/// and removed from the tail.
///
/// # Capacity
///
/// The buffer is considered 'full' when it contains `SIZE - 1` elements.
/// This design choice simplifies the empty/full detection logic.
///
/// # Type Parameters
///
/// - `T`: The type of elements stored (must be `Copy`)
/// - `SIZE`: The size of the backing array (usable capacity is `SIZE - 1`)
///
/// # Examples
///
/// ```
/// use planck_noalloc::ringbuf::RingBuf;
///
/// let mut buf = RingBuf::<i32, 16>::new();
///
/// buf.push(10);
/// buf.push(20);
/// buf.push(30);
///
/// assert_eq!(buf.pop(), Some(10));
/// assert_eq!(buf.pop(), Some(20));
/// assert_eq!(buf.len(), 1);
/// ```
#[derive(Clone, Copy)]
pub struct RingBuf<T: Copy, const SIZE: usize> {
    buf: [MaybeUninit<T>; SIZE],
    head: usize,
    tail: usize,
}

impl<T, const N: usize> Default for RingBuf<T, N>
where
    T: Copy,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> RingBuf<T, N>
where
    T: core::marker::Copy,
{
    /// The total size of the backing array. The usable capacity is `SIZE - 1`.
    pub const SIZE: usize = N;

    /// Create a new ringbuf with no data inside
    ///
    /// This method does not allocate memory.
    ///
    /// # Example
    /// ```
    /// use planck_noalloc::ringbuf::RingBuf;
    ///
    /// // Create an empty ringbuf
    /// let ringbuf = RingBuf::<u8, 8>::new();
    /// assert!(ringbuf.is_empty());
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: [const { MaybeUninit::uninit() }; N],
            head: 0,
            tail: 0,
        }
    }

    /// Returns true if the ring buffer is empty
    ///
    /// # Example
    /// ```
    /// use planck_noalloc::ringbuf::RingBuf;
    ///
    /// // Create an empty ringbuf
    /// let mut ringbuf = RingBuf::<u8, 8>::new();
    /// assert!(ringbuf.is_empty());
    /// ringbuf.push(42);
    /// assert!(!ringbuf.is_empty());
    /// ````
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        // Because we only supported SIZE-1 as maximum capacity,
        // buf is empty when head == tail
        self.head == self.tail
    }

    /// Returns the length of the used buffer
    ///
    /// # Example
    /// ```
    /// use planck_noalloc::ringbuf::RingBuf;
    ///
    /// // Create an empty ringbuf
    /// let mut ringbuf = RingBuf::<u8, 8>::new();
    /// assert_eq!(ringbuf.len(), 0);
    /// ringbuf.push(1);
    /// ringbuf.push(2);
    /// ringbuf.push(3);
    /// assert_eq!(ringbuf.len(), 3);
    /// # let popped =
    /// ringbuf.pop();
    /// # assert_eq!(popped, Some(1));
    /// assert_eq!(ringbuf.len(), 2);
    /// ```
    #[must_use]
    pub const fn len(&self) -> usize {
        (self.head + Self::SIZE - self.tail) % Self::SIZE
    }

    /// Returns true if the buffer is full
    ///
    /// # Example
    /// ```
    /// use planck_noalloc::ringbuf::RingBuf;
    ///
    /// // Create an empty ringbuf
    /// let mut ringbuf = RingBuf::<u8, 8>::new();
    /// for i in 0..6 {
    ///     ringbuf.push(i);
    /// }
    /// assert!(!ringbuf.is_full());
    /// ringbuf.push(6);
    /// assert!(ringbuf.is_full());
    /// ````
    #[must_use]
    pub const fn is_full(&self) -> bool {
        (self.head + 1) % N == self.tail
    }

    /// Returns the maximum number of elements that can be stored in the ringbuf
    /// This is calculated as the size subtracted 1
    #[must_use]
    pub const fn max_capacity(&self) -> usize {
        Self::SIZE - 1
    }

    /// Pushes (or enqueues) an element on the ring buffer
    ///
    /// # Errors
    /// Returns an error with the pushed value if the ringbuf is full
    pub fn try_push(&mut self, x: T) -> Result<(), T> {
        if self.is_full() {
            return Err(x);
        }

        // SAFETY: We checked that it isn't full
        unsafe { self.push_unchecked(x) };
        Ok(())
    }

    /// Pushes (or enqueues) an element on the ring buffer
    ///
    /// # Panics
    /// Panics if the ringbuf is full. If this is not what you want, see [`RingBuf::try_push`] or
    /// [`RingBuf::push_unchecked`].
    pub fn push(&mut self, x: T) {
        assert!(self.try_push(x).is_ok(), "ringbuf is full");
    }

    /// Pushes (or enqueues) an element on the ring buffer
    ///
    /// # Safety
    /// This does not check if it is out of bounds, which may cause data to be overwritten
    pub unsafe fn push_unchecked(&mut self, x: T) {
        self.buf[self.head].write(x);
        self.head = (self.head + 1) % N;
    }

    /// Pops (or dequeues) an element off the ring buffer
    ///
    /// Returns none if the ringbuf is empty
    #[must_use]
    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            return None;
        }

        // SAFETY: The element at `self.tail` is initialized because the buffer
        // is not empty (head != tail), and all elements between tail and head
        // were written by previous push operations.
        let x = unsafe { self.buf[self.tail].assume_init_read() };
        self.buf[self.tail] = MaybeUninit::uninit();
        self.tail = (self.tail + 1) % N;
        Some(x)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn test_is_empty() {
        let buf = RingBuf::<u8, 1024>::new();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_push() {
        let mut buf = RingBuf::<u8, 1024>::new();
        buf.try_push(15).unwrap();
        buf.try_push(42).unwrap();
        assert_eq!(unsafe { buf.buf[0].assume_init_read() }, 15);
        assert_eq!(unsafe { buf.buf[1].assume_init_read() }, 42);
    }

    #[test]
    fn test_push_rollover() {
        let mut buf = RingBuf::<u8, 1024>::new();
        assert_eq!(buf.max_capacity(), 1023);
        for i in 0..1023 {
            let res = buf.try_push((i % 255) as u8);
            assert!(res.is_ok());
        }
        assert!(buf.try_push(1).is_err());
        assert_eq!(buf.len(), 1023);

        // Now we pop one
        let res = buf.pop();
        assert!(res.is_some());
        assert_eq!(res.unwrap(), 0);
        assert_eq!(buf.len(), 1022);
        let res = buf.try_push(1);
        assert!(res.is_ok());
        assert_eq!(buf.len(), 1023);
        assert!(buf.is_full());
    }

    #[test]
    fn pop_empty() {
        let mut buf = RingBuf::<u8, 8>::new();
        assert_eq!(buf.pop(), None);
    }

    #[test]
    fn push_pop_fifo_order() {
        let mut buf = RingBuf::<u8, 8>::new();
        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert_eq!(buf.pop(), Some(1));
        assert_eq!(buf.pop(), Some(2));
        assert_eq!(buf.pop(), Some(3));
        assert_eq!(buf.pop(), None);
    }

    #[test]
    fn len_tracking() {
        let mut buf = RingBuf::<u8, 8>::new();
        assert_eq!(buf.len(), 0);
        buf.push(1);
        assert_eq!(buf.len(), 1);
        buf.push(2);
        assert_eq!(buf.len(), 2);
        let _ = buf.pop();
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn is_full_check() {
        let mut buf = RingBuf::<u8, 4>::new();
        assert!(!buf.is_full());
        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert!(buf.is_full());
    }

    #[test]
    fn max_capacity_value() {
        let buf = RingBuf::<u8, 8>::new();
        assert_eq!(buf.max_capacity(), 7);
    }

    #[test]
    fn wrap_around_multiple_times() {
        let mut buf = RingBuf::<u8, 4>::new();
        // Capacity is 3. Do multiple wrap-arounds.
        for round in 0u8..5 {
            buf.push(round * 3);
            buf.push(round * 3 + 1);
            buf.push(round * 3 + 2);
            assert_eq!(buf.pop(), Some(round * 3));
            assert_eq!(buf.pop(), Some(round * 3 + 1));
            assert_eq!(buf.pop(), Some(round * 3 + 2));
            assert!(buf.is_empty());
        }
    }

    #[test]
    #[should_panic(expected = "ringbuf is full")]
    fn push_panics_when_full() {
        let mut buf = RingBuf::<u8, 4>::new();
        buf.push(1);
        buf.push(2);
        buf.push(3);
        buf.push(4); // should panic
    }
}
