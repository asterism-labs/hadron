# Memory Management

The Hadron kernel's memory management subsystem lives primarily in
`kernel/hadron-kernel/src/mm/` with supporting types in `addr.rs` and
`paging.rs`. It covers physical frame allocation, virtual address space
management, the kernel heap, per-process address spaces, and a zone allocator
for high-frequency small allocations.

## Architecture Overview

Memory management is organized into several cooperating layers:

```
                  +--------------------------+
                  |       Zone Allocator     |  (mm/zone.rs)
                  |  fixed-size slab caches  |
                  +-------------|------------+
                                |
+----------------+   +---------|----------+
| Kernel Heap    |   |                    |
| LinkedList     |<--|   VMM (mm/vmm.rs)  |
| Allocator      |   |  page table mgmt  |
| (mm/heap.rs)   |   |  region allocators |
+----------------+   +---------|----------+
                                |
                  +-------------|------------+
                  |  PMM (mm/pmm.rs)         |
                  |  bitmap frame allocator  |
                  +-------------|------------+
                                |
                  +-------------|------------+
                  |  HHDM (mm/hhdm.rs)       |
                  |  phys-to-virt conversion |
                  +--------------------------+
```

Initialization proceeds bottom-up during boot:

1. **HHDM** -- store the bootloader-provided offset
2. **PMM** -- build the bitmap from the memory map
3. **VMM** -- create the page mapper, compute the virtual layout
4. **Heap** -- map initial heap pages, initialize the linked-list allocator
5. **Zone allocator** -- available immediately (lazily allocates pages)

## Typed Address Wrappers

Source: `addr.rs`, `paging.rs`

All addresses in the kernel are wrapped in newtypes that prevent accidental
mixing of virtual and physical values at compile time.

### `VirtAddr`

A 64-bit canonical virtual address (`repr(transparent)` over `u64`). On x86_64
with 4-level paging, bits 48..63 must be a sign-extension of bit 47.
Construction enforces this:

- `VirtAddr::new(addr)` -- panics in debug mode if `addr` is not canonical.
- `VirtAddr::new_truncate(addr)` -- silently sign-extends from bit 47.
- `VirtAddr::new_unchecked(addr)` -- unsafe, no validation.

Provides page-table index extraction methods for x86_64 (`pml4_index`,
`pdpt_index`, `pd_index`, `pt_index`) and aarch64 (`l1_index`, `l2_index`,
`l3_index`), plus alignment helpers (`align_up`, `align_down`, `is_aligned`,
`page_offset`).

### `PhysAddr`

A 64-bit physical address masked to the 52-bit physical address space
(`PHYS_ADDR_MASK = 0x000F_FFFF_FFFF_FFFF`). Provides the same `new`,
`new_truncate`, and alignment methods as `VirtAddr`.

### `Page<S>` and `PhysFrame<S>`

Generic wrappers parameterized over a `PageSize` trait. Three sizes are
defined:

| Type | Constant | Size |
|------|----------|------|
| `Size4KiB` | 4096 | 4 KiB |
| `Size2MiB` | 0x20_0000 | 2 MiB |
| `Size1GiB` | 0x4000_0000 | 1 GiB |

`Page<S>` wraps a page-aligned `VirtAddr`; `PhysFrame<S>` wraps a
frame-aligned `PhysAddr`. Both provide `containing_address` (aligns down),
`from_start_address` (returns `Err(AddressNotAligned)` if misaligned), and
range iterators (`PageRange<S>`, `PhysFrameRange<S>`).

## Physical Memory Manager (PMM)

Source: `mm/pmm.rs`

### Bitmap Allocator

The PMM uses a bitmap where each bit represents one 4 KiB frame. A set bit
(1) means allocated or reserved; a clear bit (0) means free. The bitmap is
stored in HHDM-accessible memory and managed by `BitmapAllocator`.

Internal state is held in `BitmapAllocatorInner`, protected by a `SpinLock`:

```rust
struct BitmapAllocatorInner {
    bitmap: *mut u64,        // pointer to bitmap words in HHDM
    total_frames: usize,     // total frames tracked
    bitmap_words: usize,     // number of u64 words
    free_count: usize,       // currently free frames
    search_hint: usize,      // word index hint for next scan
}
```

### Initialization

`BitmapAllocator::new(regions, hhdm_offset)` performs these steps:

1. Find the highest usable physical address to determine bitmap size.
2. Calculate the number of frames, bitmap words, and bitmap byte size.
3. Find the first usable region large enough to hold the bitmap.
4. Map the bitmap via HHDM and set all bits to 1 (all reserved).
5. Clear bits for frames in usable regions (mark free).
6. Re-set bits for the bitmap's own frames (they are now in use).

### Allocation

`allocate_frame` scans from `search_hint`, wrapping around. For each u64
word, it checks if any bit is zero using `(!word).trailing_zeros()`, which
compiles to the TZCNT/BSF instruction on x86_64, giving efficient O(1)
per-word scanning.

`allocate_frames(count)` finds `count` contiguous free frames via a linear
scan that tracks run length. Entire-word checks (`word == 0` or
`word == u64::MAX`) skip 64 frames at a time.

### Deallocation

`deallocate_frame` and `deallocate_frames` clear the corresponding bits and
update `free_count`. The `search_hint` is moved back if the freed word is
before the current hint, improving allocation locality.

Debug-mode assertions detect double-free errors.

### Traits

The module defines two unsafe traits in `mm/mod.rs`:

- `FrameAllocator<S: PageSize>` -- `allocate_frame() -> Option<PhysFrame<S>>`
- `FrameDeallocator<S: PageSize>` -- `deallocate_frame(PhysFrame<S>)`

`BitmapFrameAllocRef<'a>` is a thin wrapper around `&BitmapAllocator` that
implements both traits, bridging the interior-mutability API with the
`&mut self` trait interface.

### Global Access

The PMM is stored as a `SpinLock<Option<BitmapAllocator>>` static. Access is
through:

- `pmm::with(|pmm| ...)` -- panics if not initialized.
- `pmm::try_with(|pmm| ...)` -- returns `None` if the lock is held or
  the PMM is uninitialized (safe for use in fault handlers).

## Higher Half Direct Map (HHDM)

Source: `mm/hhdm.rs`

The bootloader (Limine) maps all of physical memory at a fixed virtual offset
in the higher half. This module stores that offset globally so any code can
convert between physical and virtual addresses.

```rust
static HHDM_OFFSET: AtomicU64 = AtomicU64::new(u64::MAX); // sentinel
```

- `hhdm::init(offset)` -- called once during early boot; uses
  `compare_exchange` to detect double-init.
- `hhdm::offset()` -- returns the offset; panics if called before `init`.
- `hhdm::phys_to_virt(phys)` -- adds the HHDM offset to produce a `VirtAddr`.
- `hhdm::virt_to_phys(virt)` -- subtracts the offset to recover a `PhysAddr`.

The HHDM is central to the kernel's design: the PMM bitmap, page tables, and
any physical frame that needs to be read or written are accessed through this
mapping rather than through temporary mappings.

## Virtual Memory Manager (VMM)

Source: `mm/vmm.rs`

### `Vmm<M>`

The VMM is generic over a `PageMapper<Size4KiB> + PageTranslator`
implementation. On x86_64, the concrete type is:

```rust
type KernelMapper = crate::arch::x86_64::paging::PageTableMapper;
type KernelVmm = Vmm<KernelMapper>;
```

The `Vmm` struct holds:

- `root_phys: PhysAddr` -- physical address of the PML4.
- `mapper: M` -- architecture-specific page table walker.
- `layout: MemoryLayout` -- the kernel's virtual address space layout.
- `heap_alloc: RegionAllocator` -- bump allocator for the heap region.
- `stacks_alloc: FreeRegionAllocator<256>` -- allocator for kernel stacks.
- `mmio_alloc: FreeRegionAllocator<128>` -- allocator for MMIO regions.

### Key Operations

**Heap growth** (`grow_heap`): Allocates virtual pages from `heap_alloc`,
obtains physical frames from the PMM, maps them with `WRITABLE | GLOBAL`
flags, and zeroes each page. The initial heap is 4 MiB (`INITIAL_HEAP_SIZE`).

**Kernel stack allocation** (`alloc_kernel_stack`): Allocates a 68 KiB region
from `stacks_alloc` (one 4 KiB guard page + 64 KiB / 16 pages of stack).
The guard page is left unmapped to catch stack overflows. Returns a
`KernelStack` that implements `Drop` -- the cleanup callback unmaps pages and
frees frames.

**MMIO mapping** (`map_mmio`): Allocates virtual space from `mmio_alloc`,
maps the specified physical range with `WRITABLE | GLOBAL | CACHE_DISABLE`
flags. Returns an `MmioMapping` with RAII cleanup.

**Page operations**: `map_page`, `unmap_page`, and `translate` provide
low-level access to the page mapper.

### Global Access

Like the PMM, the VMM is stored in a `SpinLock<Option<KernelVmm>>`. Access
is through `vmm::with_vmm(|vmm| ...)` and `vmm::try_with_vmm(...)`.

Convenience functions wrap common patterns:

- `vmm::map_initial_heap()` -- acquires both VMM and PMM locks.
- `vmm::grow_heap(min_bytes)` -- called by the heap's growth callback.
- `vmm::map_mmio_region(phys, size)` -- acquires both locks, returns the
  virtual base address.

## Page Mapper Interface

Source: `mm/mapper.rs`

The mapper layer abstracts architecture-specific page table manipulation
behind two unsafe traits:

**`PageMapper<S: PageSize>`** -- provides `map`, `unmap`, and `update_flags`.
Each returns a `MapFlush` that must be explicitly handled:

- `.flush()` -- performs a TLB shootdown for the page.
- `.ignore()` -- opts out (e.g., fresh mappings not yet in the TLB).
- Dropping without calling either will auto-flush.

**`PageTranslator`** -- provides `translate_addr(root, virt) -> Option<PhysAddr>`,
walking the page table to resolve any page size.

**`MapFlags`** is a `bitflags` type with five architecture-independent flags:

| Flag | Bit | Description |
|------|-----|-------------|
| `WRITABLE` | 0 | Page is writable |
| `EXECUTABLE` | 1 | Page is executable |
| `USER` | 2 | Accessible from user mode |
| `GLOBAL` | 3 | Not flushed on CR3 switch |
| `CACHE_DISABLE` | 4 | Caching disabled (for MMIO) |

## Kernel Heap

Source: `mm/heap.rs`

### `LinkedListAllocator`

The kernel heap implements `GlobalAlloc` using a first-fit free-list allocator.
Free blocks are stored in an intrusive linked list sorted by address:

```rust
struct FreeBlock {
    size: usize,          // total block size including header
    next: *mut FreeBlock,  // next free block (address-sorted)
}
```

Key characteristics:

- **Minimum block size**: 32 bytes (must fit a `FreeBlock` header).
- **Alignment**: all blocks are aligned to at least 16 bytes (`BLOCK_ALIGN`).
- **First-fit allocation**: walks the free list for the first block that fits.
  If the block is larger than needed, the remainder is split off (if >= 32
  bytes) and returned to the free list.
- **Coalescing on dealloc**: freed blocks are inserted in address order and
  merged with adjacent neighbors immediately, preventing fragmentation.
- **Growth callback**: when allocation fails, the allocator calls a registered
  `grow_fn` that requests at least 64 KiB of new pages from the VMM/PMM.
  The lock is released before calling the callback to avoid deadlock with the
  PMM lock.

### Initialization Flow

1. `heap::init()` calls `vmm::map_initial_heap()` to map 4 MiB of pages.
2. `init_raw(heap_start, heap_size)` creates a single `FreeBlock` spanning
   the entire mapped region.
3. `register_grow_fn(grow_callback)` registers the VMM growth function.

The allocator is declared as `#[global_allocator]` (conditional on
`target_os = "none"`).

## Kernel Address Space Layout

Source: `mm/layout.rs`

The `MemoryLayout` struct describes the kernel's virtual address space.
All dynamic regions are defined as constant offsets from a `regions_base`
(default `0xFFFF_C000_0000_0000`), making the layout KASLR-ready.

| Region | Offset from base | Max size | Purpose |
|--------|-----------------|----------|---------|
| Heap | +0 | 2 TiB | Kernel heap |
| Stacks | +8 TiB | 512 GiB | Kernel stacks with guard pages |
| MMIO | +16 TiB | 1 TiB | Device MMIO mappings |
| Per-CPU | +32 TiB | 1 TiB | Per-CPU data |
| vDSO | +48 TiB | 2 MiB | vDSO/VVAR pages |

The kernel image itself is at a fixed address (`0xFFFF_FFFF_8000_0000`,
max 128 MiB), and the HHDM base is provided by the bootloader (typically
`0xFFFF_8000_0000_0000`).

`VirtRegion` is a simple `(base: VirtAddr, max_size: u64)` pair with a
`contains(addr)` check. `MemoryLayout::identify_region(addr)` returns a
`FaultRegion` enum indicating which region a faulting address belongs to --
used by the page fault handler to produce meaningful diagnostics.

## Virtual Address Region Allocators

Source: `mm/region.rs`

Two allocators manage virtual address ranges within the regions defined by
`MemoryLayout`:

### `RegionAllocator` (bump-only)

Used for the kernel heap. Advances a cursor forward; never deallocates.
Allocations are page-aligned.

### `FreeRegionAllocator<const N: usize>`

Used for kernel stacks and MMIO regions, which require deallocation.
Combines a bump allocator with a sorted free-list backed by a fixed-capacity
`ArrayVec<FreeRange, N>`:

- **Allocate**: first-fit scan of the free list, falling back to bumping the
  watermark.
- **Deallocate**: binary search for the insertion point, coalesce with
  predecessor and/or successor. If the freed range is at the watermark, the
  watermark is retracted (and chained retractions follow).
- **Capacity**: the stacks allocator uses `N=256`, MMIO uses `N=128`. If the
  free list fills up and the range cannot be coalesced, deallocation returns
  `RegionAllocError::FreeListFull`.

## Address Spaces

Source: `mm/address_space.rs`

Each user process owns an `AddressSpace<M>`, which holds:

- A freshly allocated PML4 frame (`root_phys`).
- A `PageMapper` / `PageTranslator` (shared implementation, knows HHDM offset).
- A `FrameDeallocFn` callback for freeing the PML4 frame on drop.

Construction (`AddressSpace::new_user`) allocates a 4 KiB PML4 frame, zeroes
the lower half (entries 0--255, user space), and copies the upper half
(entries 256--511) from the kernel's PML4. This ensures all address spaces
share the kernel mappings.

Key methods:

- `map_user_page(page, frame, flags, alloc)` -- maps a page with the `USER`
  flag always set.
- `unmap_user_page(page)` -- unmaps and flushes, returns the freed frame.
- `root_phys()` -- returns the PML4 physical address for loading into CR3.
- `translate(virt)` -- walks the page table.

The `Drop` implementation frees the PML4 frame via the stored callback.

## Zone Allocator

Source: `mm/zone.rs`

The zone allocator provides fast, fixed-size block allocation for
high-frequency, short-lived kernel objects. It maintains 8 zones with
power-of-two size classes:

| Zone | Block size |
|------|-----------|
| 0 | 32 bytes |
| 1 | 64 bytes |
| 2 | 128 bytes |
| 3 | 256 bytes |
| 4 | 512 bytes |
| 5 | 1024 bytes |
| 6 | 2048 bytes |
| 7 | 4096 bytes |

Each zone maintains an intrusive free list (`FreeZoneBlock`). When a zone's
free list is empty, it requests a new 4 KiB page from the PMM via HHDM and
carves it into blocks of the zone's size. Allocations larger than 4096 bytes
are rejected with `ZoneAllocError::TooLarge`.

The global instance is accessed through `zone_alloc(layout)` and
`zone_dealloc(ptr, layout)`.

## Error Types

Source: `mm/mod.rs`

Two error enums cover the PMM and VMM:

**`PmmError`**: `OutOfMemory`, `InvalidFrame`, `AlreadyInitialized`,
`NoBitmapRegion`.

**`VmmError`**: `RegionExhausted`, `OutOfMemory`, `NotMapped`,
`AlreadyMapped`, `SizeMismatch`.

## Summary

The memory management subsystem is designed around a few key principles:

- **Type safety**: `VirtAddr`, `PhysAddr`, `Page<S>`, and `PhysFrame<S>`
  prevent mixing address kinds and page sizes at compile time.
- **Architecture independence**: the `PageMapper` and `PageTranslator` traits
  let the VMM work without knowing whether it is running on x86_64 or aarch64.
- **RAII cleanup**: `KernelStack`, `MmioMapping`, and `AddressSpace` all
  implement `Drop` to unmap pages and free frames.
- **Interior mutability with spin locks**: both the PMM and VMM are stored as
  global `SpinLock<Option<T>>` singletons, with `try_*` variants for use in
  fault handlers where the lock may already be held.
- **Growable heap**: the linked-list allocator requests new pages on demand,
  allowing the heap to expand from its initial 4 MiB without a fixed upper
  bound (up to the 2 TiB region limit).
