//! Minimal bump-with-freelist heap allocator for userspace servers.
//!
//! Requests memory from the kernel via `sys_mem_map` in 64 KiB chunks.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;

use hadron_syscall::constants::{MAP_ANONYMOUS, PROT_READ, PROT_WRITE};
use hadron_syscall::wrappers;

const CHUNK_SIZE: usize = 64 * 1024;
const MIN_ALIGN: usize = 16;

struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

struct HeapInner {
    bump: *mut u8,
    bump_end: *mut u8,
    free_list: *mut FreeBlock,
}

pub struct UserHeap {
    inner: UnsafeCell<HeapInner>,
}

// SAFETY: Hadron userspace processes are single-threaded.
unsafe impl Sync for UserHeap {}

impl UserHeap {
    pub const fn new() -> Self {
        Self {
            inner: UnsafeCell::new(HeapInner {
                bump: core::ptr::null_mut(),
                bump_end: core::ptr::null_mut(),
                free_list: core::ptr::null_mut(),
            }),
        }
    }
}

const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

fn grow(inner: &mut HeapInner, min_size: usize) -> bool {
    let size = if min_size > CHUNK_SIZE {
        align_up(min_size, 4096)
    } else {
        CHUNK_SIZE
    };

    let ret = wrappers::sys_mem_map(0, size, PROT_READ | PROT_WRITE, MAP_ANONYMOUS, usize::MAX);
    if ret > 0 {
        inner.bump = ret as *mut u8;
        inner.bump_end = unsafe { (ret as *mut u8).add(size) };
        true
    } else {
        false
    }
}

unsafe impl GlobalAlloc for UserHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let inner = unsafe { &mut *self.inner.get() };
        let align = layout.align().max(MIN_ALIGN);
        let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), align);

        // Try free list (first-fit).
        let mut prev: *mut *mut FreeBlock = &mut inner.free_list;
        let mut current = inner.free_list;
        while !current.is_null() {
            let block = unsafe { &mut *current };
            let block_addr = current as usize;
            let aligned_addr = align_up(block_addr, align);
            let padding = aligned_addr - block_addr;

            if block.size >= size + padding {
                unsafe { *prev = block.next };
                return aligned_addr as *mut u8;
            }
            prev = unsafe { &mut (*current).next };
            current = block.next;
        }

        // Try bump allocation.
        let bump_addr = align_up(inner.bump as usize, align);
        let bump_end = bump_addr + size;
        if bump_end <= inner.bump_end as usize {
            inner.bump = bump_end as *mut u8;
            return bump_addr as *mut u8;
        }

        // Need a new chunk.
        if !grow(inner, size + align) {
            return core::ptr::null_mut();
        }

        let bump_addr = align_up(inner.bump as usize, align);
        let bump_end = bump_addr + size;
        inner.bump = bump_end as *mut u8;
        bump_addr as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let inner = unsafe { &mut *self.inner.get() };
        let align = layout.align().max(MIN_ALIGN);
        let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), align);

        let block = ptr as *mut FreeBlock;
        unsafe {
            (*block).size = size;
            (*block).next = inner.free_list;
        }
        inner.free_list = block;
    }
}
