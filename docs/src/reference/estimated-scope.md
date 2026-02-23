# Estimated Scope

This chapter provides rough estimates for lines of code, unsafe percentages, and key learning areas for the remaining features. These are approximations to help with planning --- actual numbers will vary.

## Completed Features

| Feature | Status |
|---------|--------|
| Async VFS & Ramfs | Complete (+ FAT, ISO9660) |
| Userspace & ELF Loading | Complete (+ lepton-init, lsh, coreutils) |
| Device Drivers | Complete (+ AHCI, e1000e, Bochs VGA, PS/2 mouse) |
| Display Infrastructure | Complete (+ `/dev/fb0` mmap and ioctl) |
| Input Handling | Complete (+ `/dev/kbd` and `/dev/mouse` raw events) |
| IPC Channels & Shared Memory | Complete (+ channels, eventfd, memfd) |
| TTY & Terminal System | Complete (+ multiple VTs, line discipline, signal dispatch) |
| IPC & Signal Handling | Complete (+ signal trampoline, process groups) |
| Threading & task_clone | Complete (+ TLS support, Arc<> sharing) |
| SMP & Per-CPU Executors | Complete (+ two-step AP boot, work stealing) |
| Network Stack - Phase 1 (ARP & ICMP) | Complete (+ async RX, zero-copy) |
| Userspace Compositor | Complete (+ window manager, protocol) |

## Remaining Features

| Feature | Approx LOC | Unsafe % | Key Learning |
|---------|-----------|----------|--------------|
| VirtIO GPU 2D Driver | ~1,000 | ~5% | VirtIO GPU protocol, resource management, hardware cursor |
| Network Stack - Phase 2 (TCP/UDP) | ~1,500 | ~5% | TCP state machine, UDP, socket syscalls |
| vDSO & Performance | ~700 | ~15% | vDSO, seqlock, TSC, futex |
| **Total remaining** | | **~3,200** | **~8%** | |

## Deferred

| Item | Approx LOC | Notes |
|------|-----------|-------|
| ext2 Filesystem | ~1,500 | Deferred — pick up when persistent storage is needed |
| OpenGL/Vulkan | N/A | Requires Mesa port (~4M LOC) — long-term aspiration |
| USB HID | ~1,500 | USB keyboard/mouse — deferred until USB host controller work |

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
| Networking | ~5% | Custom protocol implementation |
| Compositor | ~0% | Entirely userspace, pure Rust |
