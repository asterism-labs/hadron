//! Physical memory management that does not touch hardware: the memory
//! map, frame allocators, and the page table mapper.
//!
//! The kernel builds these during boot. Every structure here stores
//! physical addresses and reaches memory only through a [`DirectMap`], so
//! the host test suite can run them over ordinary heap memory.
//!
//! The mapper builds `x86_64` 4-level page tables. The aarch64 port will
//! need its own.

#![cfg_attr(not(test), no_std)]

mod bitmap;
mod bump;
mod direct_map;
mod frame;
mod mapper;
mod memory_map;

pub use bitmap::{BitmapFrameAllocator, FreeError};
pub use bump::BumpAllocator;
pub use direct_map::DirectMap;
pub use frame::{FrameAllocator, PhysFrame};
pub use mapper::{MapError, Mapper, Translation, UnmapError};
pub use memory_map::{BuildError, MemoryMap, Region, RegionKind};

#[cfg(test)]
mod testing;
