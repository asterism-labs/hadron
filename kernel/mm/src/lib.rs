//! Memory management that does not touch hardware: the memory map, frame
//! allocators, the page table mapper, and the kernel heap's allocator.
//!
//! The kernel builds these during boot. Every structure here stores
//! physical addresses and reaches memory only through a [`DirectMap`], so
//! the host test suite can run them over ordinary heap memory.
//!
//! The mapper builds `x86_64` 4-level page tables. The aarch64 port will
//! need its own.

#![cfg_attr(not(test), no_std)]

pub mod allocators;
mod direct_map;
mod frame;
mod mapper;
mod memory_map;

pub use direct_map::DirectMap;
pub use frame::{FrameAllocator, PhysFrame};
pub use mapper::{MapError, Mapper, PageMapping, Translation, UnmapError, pages};
pub use memory_map::{BuildError, MemoryMap, Region, RegionKind};

#[cfg(test)]
mod testing;
