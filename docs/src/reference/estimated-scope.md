# Estimated Scope

This chapter provides rough estimates for lines of code, unsafe percentages, and key learning areas for the remaining phases. These are approximations to help with planning --- actual numbers will vary.

## Remaining Phases

| Phase | Name | Approx LOC | Unsafe % | Key Learning |
|-------|------|-----------|----------|--------------|
| 8 | Async VFS & Ramfs | ~1,500 | ~5% | Async VFS design, inode abstraction |
| 9 | Userspace & ELF Loading | ~1,000 | ~15% | ELF format, ring 3 transition |
| 10 | Device Drivers | ~2,000 | ~10% | PCI, VirtIO, async block devices |
| 11 | IPC & Minimal Signals | ~1,200 | ~5% | Async channels, pipes, signals |
| 12 | SMP & Per-CPU Executors | ~1,000 | ~20% | AP bootstrap, per-CPU data, work stealing |
| 13 | ext2 Filesystem | ~1,500 | ~5% | Filesystem on-disk format |
| 14 | Networking | ~1,500 | ~5% | smoltcp integration, socket API |
| 15 | vDSO & Performance | ~700 | ~15% | vDSO, seqlock, TSC, futex |
| **Total remaining** | | **~10,400** | **~10%** | |

## Unsafe Distribution

The overall unsafe rate is consistent with the framekernel target. Unsafe code concentrates in the frame layer (`hadron-kernel::arch` and `mm` modules) where hardware interaction is unavoidable.

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
