//! Kernel heap — kernel glue.
//!
//! Re-exports the linked-list allocator and `#[global_allocator]` from
//! `hadron-mm`. Adds kernel-specific initialization that wires the heap
//! to the VMM/PMM for initial mapping and growth.

pub use hadron_mm::heap::*;

/// Initializes the kernel heap.
///
/// 1. Maps initial heap pages via VMM/PMM.
/// 2. Initializes the global linked-list allocator.
/// 3. Registers the growth callback.
pub fn init() {
    let (heap_start, heap_size) = super::vmm::map_initial_heap();

    unsafe {
        hadron_mm::heap::init_raw(heap_start, heap_size);
    }

    hadron_mm::heap::register_grow_fn(grow_callback);

    crate::ktrace_subsys!(
        mm,
        "heap initialized at {:#x}, size {:#x} ({} KiB)",
        heap_start,
        heap_size,
        heap_size / 1024
    );
}

/// Growth callback invoked by the heap allocator when it runs out of space.
fn grow_callback(min_bytes: usize) -> Option<(*mut u8, usize)> {
    super::vmm::grow_heap(min_bytes)
}
