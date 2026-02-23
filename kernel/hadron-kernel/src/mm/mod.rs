//! Memory management types, traits, and subsystems.
//!
//! Core data structures and algorithms live in the `hadron-mm` crate for
//! host testability. This module re-exports them and adds kernel-specific
//! glue (boot-info conversion, global VMM wiring, heap init).

// Re-export root-level items from hadron-mm.
pub use hadron_mm::{
    FrameAllocator, FrameDeallocator, PAGE_MASK, PAGE_SIZE, PhysMemoryRegion, PmmError, VmmError,
    zero_frame,
};

// Re-export submodules that don't need kernel extension.
pub use hadron_mm::address_space;
pub use hadron_mm::hhdm;
pub use hadron_mm::layout;
pub use hadron_mm::mapper;
pub use hadron_mm::region;
pub use hadron_mm::zone;

// Kernel-extended modules (re-export hadron-mm contents + add glue).
pub mod heap;
pub mod pmm;
pub mod scope;
pub mod vmm;

/// Zeroes a 4 KiB page frame using the dispatched `kernel_memzero`.
///
/// # Safety
///
/// `ptr` must point to a writable, 4 KiB-aligned region of at least
/// [`PAGE_SIZE`] bytes.
#[cfg(hadron_alt_instructions)]
pub unsafe fn kernel_zero_frame(ptr: *mut u8) {
    unsafe { crate::arch::x86_64::mem::kernel_memzero(ptr, PAGE_SIZE) };
}
