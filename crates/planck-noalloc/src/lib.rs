//! Stack-allocated data structures with similar APIs to heap-allocated types.
//!
//! This crate provides fixed-size, stack-allocated alternatives to common heap-allocated
//! data structures from the standard library. These types are useful in environments where
//! heap allocation is unavailable, restricted, or undesirable (such as embedded systems,
//! kernels, interrupt handlers, or early boot code).
//!
//! # Overview
//!
//! The crate includes the following data structures:
//!
//! - [`vec::ArrayVec`] - A fixed-capacity vector backed by a stack-allocated array
//! - [`ringbuf::RingBuf`] - A fixed-capacity circular/ring buffer for FIFO operations
//!
//! All types in this crate:
//! - Do not perform heap allocation
//! - Have a fixed maximum capacity determined at compile time
//! - Work in `no_std` environments
//! - Provide APIs similar to their standard library counterparts
//!
//! # When to Use This Crate
//!
//! Use `planck_noalloc` when:
//! - You're working in a `no_std` environment without an allocator
//! - You need predictable memory usage and performance
//! - You want to avoid heap fragmentation
//! - You're in an interrupt handler or other restricted context
//! - Maximum size is known at compile time
//!
//! # Examples
//!
//! ## Using `ArrayVec`
//!
//! ```
//! use planck_noalloc::vec::ArrayVec;
//!
//! // Create a vector that can hold up to 5 elements on the stack
//! let mut vec = ArrayVec::<i32, 5>::new();
//!
//! vec.push(1);
//! vec.push(2);
//! vec.push(3);
//!
//! assert_eq!(vec.len(), 3);
//! assert_eq!(vec[0], 1);
//!
//! for value in vec.iter() {
//!     println!("{}", value);
//! }
//! ```
//!
//! ## Using `RingBuf`
//!
//! ```
//! use planck_noalloc::ringbuf::RingBuf;
//!
//! // Create a ring buffer that can hold up to 7 elements (SIZE-1)
//! let mut buf = RingBuf::<u8, 8>::new();
//!
//! buf.push(1);
//! buf.push(2);
//! buf.push(3);
//!
//! assert_eq!(buf.pop(), Some(1));
//! assert_eq!(buf.pop(), Some(2));
//! assert_eq!(buf.len(), 1);
//! ```
//!
//! # Features
//!
//! - `std` (default): Enables std-specific features like `Error` trait implementations
//!
//! # Performance Characteristics
//!
//! All operations have the same time complexity as their heap-allocated counterparts:
//! - Push/pop: O(1)
//! - Index access: O(1)
//! - Iteration: O(n)
//!
//! However, these types avoid heap allocation overhead and have better cache locality
//! since data is stored inline on the stack.

#![no_std]
#![feature(allocator_api)]

pub mod ringbuf;
pub mod vec;
