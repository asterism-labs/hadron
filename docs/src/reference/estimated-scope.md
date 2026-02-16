# Estimated Scope

This chapter provides rough estimates for lines of code, unsafe percentages, and key learning areas for each phase. These are approximations to help with planning --- actual numbers will vary.

## Completed Work

Phases 0-6 are complete. See [Completed Work](../phases/completed-work.md) for details.

| Phase | Name | Approx LOC | Unsafe % | Key Learning |
|-------|------|-----------|----------|--------------|
| 0 | Build System & Boot Stub | ~800 | ~15% | Cross-compilation, boot protocols |
| 1 | Serial Console | ~600 | ~15% | UART, I/O ports, inline assembly |
| 2 | CPU Initialization | ~2,800 | ~25% | GDT/IDT, x86 privilege levels |
| 3 | Physical Memory | ~500 | ~20% | Physical memory layout, bitmap allocator |
| 4 | Virtual Memory & Heap | ~1,200 | ~25% | Page tables, virtual memory |
| 5 | Interrupts & Timers | ~1,000 | ~30% | ACPI, APIC, interrupt routing |
| 6 | Async Executor | ~1,000 | ~10% | Cooperative async, futures, wakers |
| **Subtotal** | | **~7,900** | **~20%** | |

## Remaining Phases

| Phase | Name | Approx LOC | Unsafe % | Key Learning |
|-------|------|-----------|----------|--------------|
| 7 | Syscall Interface | ~800 | ~20% | SYSCALL/SYSRET, ABI design |
| 8 | Async VFS & Ramfs | ~1,500 | ~5% | Async VFS design, inode abstraction |
| 9 | Userspace & ELF Loading | ~1,000 | ~15% | ELF format, ring 3 transition |
| 10 | Device Drivers | ~2,000 | ~10% | PCI, VirtIO, async block devices |
| 11 | IPC & Minimal Signals | ~1,200 | ~5% | Async channels, pipes, signals |
| 12 | SMP & Per-CPU Executors | ~1,000 | ~20% | AP bootstrap, per-CPU data, work stealing |
| 13 | ext2 Filesystem | ~1,500 | ~5% | Filesystem on-disk format |
| 14 | Networking | ~1,500 | ~5% | smoltcp integration, socket API |
| 15 | vDSO & Performance | ~700 | ~15% | vDSO, seqlock, TSC, futex |
| **Subtotal** | | **~11,200** | **~11%** | |

| | | **~19,100** | **~15%** | |

## Unsafe Distribution

The overall ~15% unsafe rate is consistent with the framekernel target. Unsafe code concentrates in the frame layer (hadron-core) where hardware interaction is unavoidable.

### High Unsafe (>20%)

These components directly interact with hardware:

| Component | Unsafe % | Why |
|-----------|----------|-----|
| APIC drivers | ~30% | MMIO register access |
| CPU init (GDT/IDT/TSS) | ~25% | CPU descriptor table setup |
| Page table mapper | ~25% | Raw page table entry writes, CR3 |
| Physical memory manager | ~20% | Raw physical address manipulation |
| Syscall entry | ~20% | Assembly stub, MSR programming |
| SMP bootstrap | ~20% | AP startup, GS base setup |

### Low Unsafe (<10%)

These components use safe APIs from the frame:

| Component | Unsafe % | Why |
|-----------|----------|-----|
| Executor / scheduler | ~10% | Waker raw pointer encoding |
| Drivers | ~10% | Use safe I/O wrappers |
| IPC / Signals | ~5% | Data structure management |
| VFS | ~5% | Pure Rust data structures |
| ext2 | ~5% | Block reads through safe async trait |
| Networking | ~5% | smoltcp handles protocol details |

## Changes from Original Estimates

The async executor model reduces unsafe code in several areas:

- **Phase 6**: No `switch_context()` naked assembly, no per-task kernel stack allocation. Unsafe percentage dropped from ~10% to ~10% (different unsafe: waker encoding instead of context switch).
- **Phase 11**: No `fork()` or CoW page fault handling. Reduced from ~1,500 LOC / ~10% unsafe to ~1,200 LOC / ~5% unsafe.
- **Phase 14**: Using `smoltcp` instead of from-scratch TCP/IP reduces LOC from ~2,500 to ~1,500 and eliminates protocol-level bugs.
