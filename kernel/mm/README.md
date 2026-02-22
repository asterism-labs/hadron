# hadron-mm

Memory management subsystem for the Hadron kernel. This crate provides the physical memory manager (PMM), virtual memory manager (VMM), kernel heap allocator, page table mapping abstractions, and virtual address space layout definitions. It is architecture-independent -- arch-specific page table implementations (e.g., x86_64 `PageTableMapper`) are injected via the `PageMapper` and `PageTranslator` traits. The crate operates in `no_std` and depends only on `hadron-core` for address types and synchronization.

## Features

- **Bitmap-based physical frame allocator** -- tracks 4 KiB frames with a bitmap stored in HHDM-mapped memory; uses word-level scanning with `trailing_zeros()` (compiles to TZCNT/BSF on x86_64) and a search hint for amortized O(1) allocation; supports both single-frame and contiguous multi-frame allocation
- **Virtual memory manager** -- manages kernel virtual address space with separate region allocators for heap (bump-only), stacks (free-list with deallocation), and MMIO (free-list with deallocation); maps pages through a pluggable `PageMapper` trait
- **Linked-list kernel heap allocator** -- first-fit free list sorted by address with immediate coalescing on dealloc; implements `GlobalAlloc` for use as `#[global_allocator]`; supports a growth callback to request more pages from the VMM/PMM when the heap is exhausted
- **HHDM (Higher Half Direct Map)** -- global offset translation between physical and virtual addresses, initialized once from the bootloader's HHDM base
- **Kernel memory layout** -- defines the virtual address regions for HHDM, heap, kernel stacks, and MMIO based on the physical address space size, ensuring non-overlapping regions in the upper half
- **Guard-page kernel stacks** -- allocates 64 KiB kernel stacks with an unmapped guard page at the bottom; RAII `KernelStack` type calls a cleanup callback on drop
- **MMIO mapping** -- maps physical device registers into the kernel MMIO region with cache-disable flags; RAII `MmioMapping` type calls a cleanup callback on drop
- **User address spaces** -- `AddressSpace` type owns a per-process PML4 with RAII cleanup of the root page table frame on drop
- **Page mapper traits** -- `PageMapper` for map/unmap operations with TLB flush tokens, `PageTranslator` for virtual-to-physical translation, and `FrameAllocator`/`FrameDeallocator` safety traits for physical frame management
- **Debug diagnostics** -- optional page poisoning on free (writes `0xDEADDEAD` pattern, verifies on re-allocation to detect use-after-free), optional heap red zones with magic cookies for buffer overflow/underflow detection, and allocation tracking with peak/live statistics
- **Free region allocator** -- generic free-list allocator with configurable capacity, used for stack and MMIO region management with both allocation and deallocation support
