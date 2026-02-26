# Memory & Allocation Systems

Hadron's memory subsystem spans two layers: the **Physical Memory Manager (PMM)** handles allocation of physical page frames, and the **Virtual Memory Manager (VMM)** provides page table management and virtual address space layout. Both are designed to work early in boot before the heap allocator is available.

Source: [`kernel/mm/src/`](https://github.com/anomalyco/hadron/blob/main/kernel/mm/src/), [`kernel/kernel/src/mm/`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/mm/)

## Physical Memory Manager (PMM)

The PMM allocates and frees physical page frames. It uses a bitmap allocator for O(1) allocation and freeing.

**Key Components:**

- **`FrameAllocator`** -- Bitmap-based allocator tracking which pages are free/in-use.
- **`FRAME_ALLOCATOR`** -- Global PMM instance, initialized during early boot from the bootloader's memory map.

**API:**

- `allocate_frame() -> PhysAddr` -- Allocates a single 4 KiB frame; panics if out of memory.
- `allocate_frames(count) -> PhysAddr` -- Allocates `count` contiguous frames.
- `deallocate_frame(addr)` -- Frees a frame back to the allocator.
- `deallocate_frames(addr, count)` -- Frees `count` contiguous frames.

**Usage:** The PMM is used to allocate frames for page tables (PML4, PDPT, PD, PT), kernel stacks, process stacks, and any heap-backed allocation.

## Virtual Memory Manager (VMM)

The VMM manages virtual address spaces and maps virtual addresses to physical frames via page tables.

**AddressSpace:**

The primary type is `AddressSpace<M>`, a generic container over a page table mapper `M` (currently `PageTableMapper` for x86_64 four-level paging).

```rust
pub struct AddressSpace<M> {
    root: PhysAddr,           // PML4 physical address
    mapper: M,                // Mapper trait implementation
    dealloc_fn: fn(PhysAddr), // Callback to free the root frame
}
```

**Kernel address space** is created at boot and shared across all processes (upper half, 0xFFFF_8000_0000_0000 and above). The kernel CR3 is saved at boot and restored when entering/exiting userspace.

**User address spaces** are created per-process. Each process gets a fresh PML4 frame with:

- **Lower half** (entries 0-255) -- zeroed, reserved for user mappings.
- **Upper half** (entries 256-511) -- copied from the kernel PML4, sharing the kernel's page table subtrees.

This design allows kernel code to run within a user process's address space (at ring 0 only) for syscall handling, without requiring a CR3 switch until returning to ring 3.

**Mapping API:**

- `map(vaddr, paddr, flags)` -- Maps a virtual address to a physical address with the specified flags (`USER`, `WRITABLE`, `EXECUTABLE`).
- `unmap(vaddr)` -- Removes a mapping.
- `translate(vaddr) -> Option<(PhysAddr, PageFlags)>` -- Translates a virtual address to physical.

The mapper allocates intermediate page table frames as needed from the PMM.

## Kernel Memory Layout

The kernel occupies the upper half of the x86_64 address space. The layout is defined in the linker script and follows this structure:

```
0xFFFF_FFFF_FFFF_FFFF (top)
    +-------------------+
    | Kernel Heap       | (grows upward, typically 4 MiB initial size)
    +-------------------+
    | (reserved gap)    |
    +-------------------+
    | HHDM (1:1 map)    | (physical memory mapped 1:1 starting at HHDM_OFFSET)
    +-------------------+
    | Per-CPU Data      | (per-CPU storage, APIC IDs, GS/FS bases, etc.)
    +-------------------+
    | Kernel Stacks     | (stack for each CPU, 64 KiB each + guard pages)
    +-------------------+
    | MMIO              | (device memory mapped here, architecture-specific)
    +-------------------+
    | vDSO/VVAR         | (virtual DSO pages for fast syscalls, seqlock)
    +-------------------+
    | Kernel Image      | (kernel code and read-only data, relocated by bootloader)
    +-------------------+
0xFFFF_8000_0000_0000 (kernel space start)
                     ...
0x0000_8000_0000_0000 (user space start)
    +-------------------+
    | User Heap         | (grows upward)
    +-------------------+
    | (mmaps, anon)     | 
    +-------------------+
    | User Stack        | (64 KiB, grows downward from 0x7FFF_FFFF_F000)
    +-------------------+
    | .rodata/.text     | (read-only + executable)
    +-------------------+
0x0000_0000_0000_0000 (user space bottom)
```

## Higher-Half Direct Map (HHDM)

The HHDM is a 1:1 mapping of all physical memory into the kernel address space, starting at `HHDM_OFFSET` (typically `0xFFFF_8000_0000_0000`). This allows the kernel to:

1. Access any physical page without requiring a temporary mapping.
2. Write to page table frames and other metadata structures without special handling.
3. Copy data between address spaces by mapping both the source and destination regions into kernel virtual space.

**Usage example:** When a process loads an ELF binary, the kernel allocates physical frames from the PMM, maps them via the user's address space, then writes file data to them via HHDM pointers without needing temporary kernel mappings.

## Kernel Heap

The kernel heap is allocated on-demand using a buddy allocator implemented in `kernel/mm/src/heap.rs`. The heap is protected by a `SpinLock` and uses `alloc::alloc::GlobalAlloc` for integration with Rust's `Box`, `Vec`, `String`, etc.

**Initialization:** The heap is initialized at boot by reserving a 4 MiB region in the kernel address space and registering it with the buddy allocator.

**Protection:** Access to the heap during early boot and interrupt handlers is carefully controlled via `hadron_lock_debug` to prevent deadlocks.

## Per-CPU Storage

Each CPU has a small amount of pre-allocated storage in the upper kernel address space (per-CPU region). This is accessed via the `CpuLocal<T>` wrapper, which indexes by CPU ID.

**Key per-CPU state:**

- Executor state (task map, ready queues)
- Current process and trap reason
- User context snapshots
- Kernel stack pointer
- GS base (pointing to the per-CPU data structure itself)

The GS base is set up during AP bootstrap via `IA32_GS_BASE` MSR, allowing efficient access to per-CPU data via `GS:[offset]` in both Rust and assembly code.

## Address Space and CR3 Management

**User address space switching:**

When entering userspace:
1. Load `process.user_cr3` into CR3.
2. Disable interrupts (`cli`).
3. Save kernel GS base to `IA32_KERNEL_GS_BASE` and zero `IA32_GS_BASE`.
4. Execute `iretq` to ring 3.

When returning to kernel:
1. An interrupt or fault occurs.
2. Hardware saves RIP/RSP/RFLAGS/CS/SS to the interrupt stack.
3. The `swapgs` instruction restores the kernel GS base.
4. Restore the kernel CR3 from the global `KERNEL_CR3` atomic.
5. Restore `IA32_GS_BASE` and `IA32_KERNEL_GS_BASE` so both point to per-CPU data.

**Process creation and cleanup:**

Each process's `AddressSpace` is wrapped in `Arc` and dropped when the last reference is released, triggering the dealloc callback to free the PML4 frame back to the PMM.

## Memory Protection Flags

The `PageFlags` enum controls page permissions:

- **`USER`** -- User mode can access (ring 3 execution).
- **`WRITABLE`** -- Page is writable (ring 3 write).
- **`EXECUTABLE`** -- Page is executable (x86_64 NX bit control).
- **`WRITE_COMBINE`** -- Write-combining memory type for device memory (WC MMIO).
- **`WRITE_BACK`** -- Write-back memory type (default for normal RAM).

These flags are translated to the appropriate x86_64 page table entry bits during mapping.
