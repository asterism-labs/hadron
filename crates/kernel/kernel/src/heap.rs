//! The kernel heap: the global allocator behind `alloc`.
//!
//! [`init`] maps [`HEAP_SIZE`] bytes at [`HEAP`] with frames from the
//! frame allocator. The heap does not grow, so running out panics through
//! `alloc`'s allocation error handler. Allocations before [`init`] fail
//! the same way.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};

use mm::Heap;

use crate::memory::{self, HEAP, HEAP_SIZE};
use crate::sync::{Ctx, IrqSpinLock, LockBefore, levels};

/// The global allocator's state. `GlobalAlloc` cannot take a token, so it
/// locks without one, and the heap is last in the lock order.
struct KernelHeap(IrqSpinLock<levels::Heap, Option<Heap>>);

#[global_allocator]
static KERNEL_HEAP: KernelHeap = KernelHeap(IrqSpinLock::new(None));

// SAFETY: `Heap` hands out disjoint blocks with the requested alignment and
// takes back only what it handed out, and the lock serializes callers.
unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.0
            .lock_without_ctx()
            .as_mut()
            .and_then(|heap| heap.allocate(layout))
            .map_or(ptr::null_mut(), NonNull::as_ptr)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let mut heap = self.0.lock_without_ctx();
        let heap = heap
            .as_mut()
            .expect("memory was allocated before it is freed");
        let ptr = NonNull::new(ptr).expect("the allocator never returns null on success");
        // SAFETY: `GlobalAlloc` callers free only what `alloc` returned,
        // with the same layout.
        unsafe { heap.deallocate(ptr, layout) };
    }
}

/// Maps the heap and makes it the global allocator.
///
/// # Panics
///
/// Panics if called twice or if there are not enough frames.
pub(crate) fn init<P: LockBefore<levels::KernelTables>>(ctx: &mut Ctx<'_, P>) {
    memory::map_new_pages(ctx, HEAP, HEAP_SIZE).expect("frames for the heap");
    let mut heap = KERNEL_HEAP.0.lock(ctx);
    assert!(heap.is_none(), "heap::init called twice");
    #[allow(clippy::cast_possible_truncation)] // fits on a 64-bit target
    // SAFETY: the range was just mapped, read-write, and only the heap
    // uses it.
    let new = unsafe { Heap::new(HEAP.as_mut_ptr(), HEAP_SIZE as usize) };
    *heap = Some(new);
}

/// Free bytes in the heap, or 0 before [`init`].
pub(crate) fn free_bytes<P: LockBefore<levels::Heap>>(ctx: &mut Ctx<'_, P>) -> usize {
    KERNEL_HEAP.0.lock(ctx).as_ref().map_or(0, Heap::free_bytes)
}

ktest::ktest! {
    fn box_and_vec_use_the_heap() {
        use alloc::boxed::Box;
        use alloc::vec::Vec;

        let boxed = Box::new(0x1234_u64);
        let addr = arch_x86_64::VirtAddr::from_ptr(&raw const *boxed);
        assert!(addr >= HEAP && addr < HEAP + HEAP_SIZE);
        let mut v = Vec::new();
        for i in 0..10_000_u32 {
            v.push(i);
        }
        assert_eq!(v.iter().map(|&i| u64::from(i)).sum::<u64>(), 49_995_000);
    }
}

ktest::ktest! {
    fn freed_memory_returns_to_the_heap() {
        use alloc::vec::Vec;

        let mut ctx = Ctx::root();
        let before = free_bytes(&mut ctx);
        let big: Vec<u8> = alloc::vec![0xa5; 1 << 20];
        assert!(free_bytes(&mut ctx) <= before - (1 << 20));
        assert!(big.iter().all(|&b| b == 0xa5));
        drop(big);
        assert_eq!(free_bytes(&mut ctx), before);
    }
}
