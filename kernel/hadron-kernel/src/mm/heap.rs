//! Kernel heap initialization and growth callback.

/// Initializes the kernel heap.
///
/// 1. Maps initial heap pages via VMM/PMM.
/// 2. Initializes the global linked-list allocator.
/// 3. Registers the growth callback.
pub fn init() {
    let (heap_start, heap_size) = super::vmm::map_initial_heap();

    unsafe {
        hadron_core::mm::heap::init(heap_start, heap_size);
    }

    hadron_core::mm::heap::register_grow_fn(grow_callback);
}

/// Growth callback invoked by the heap allocator when it runs out of space.
fn grow_callback(min_bytes: usize) -> Option<(*mut u8, usize)> {
    super::vmm::grow_heap(min_bytes)
}
