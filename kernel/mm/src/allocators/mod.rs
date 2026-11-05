//! Allocators: frame allocators for physical memory, and the allocator
//! behind the kernel heap.
//!
//! - [`BumpAllocator`] hands out frames from one range in order and never
//!   frees. Boot uses it before the bitmap exists.
//! - [`BitmapFrameAllocator`] tracks every frame with one bit.
//! - [`LinkedListAllocator`] hands out byte ranges from a free list over
//!   one fixed range. The kernel heap uses it.

mod bitmap;
mod bump;
mod linked_list;

pub use bitmap::{BitmapFrameAllocator, FreeError};
pub use bump::BumpAllocator;
pub use linked_list::LinkedListAllocator;
