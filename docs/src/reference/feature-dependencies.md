# Feature Dependencies

This chapter shows how the remaining features depend on each other. The foundational features (Boot, PMM, VMM/Heap, Interrupts/APIC, Executor, Syscall Interface, Async VFS & Ramfs, Userspace & ELF Loading, Device Drivers, IPC & Minimal Signals, SMP & Per-CPU Executors) are all complete.

## Dependency Graph

```
Completed Features
    |
    +---> Input & Display ---> VirtIO GPU ---> Compositor
    |                                |
    |                                +--- (Compositor also needs IPC & Minimal Signals)
    |
    +---> Networking (builds on existing ARP/ICMP/IPv4)
    |
    +---> vDSO (needs Userspace & ELF Loading)
    |
    +---> Graphics Stack:
          |
          +---> mprotect ─────────────────────────────────────┐
          +---> Dynamic devfs ─────────────────────────────────┤
          +---> sysfs ─────────────────────────────────────────┤
          +---> sys_query extensions ──────────────────────────┤
          +---> poll() + pthreads (libc) ──────────────────────┼──> Mesa Port ──┐
          +---> Unix Domain Sockets ───> Wayland Subset ───────┘               │
          |                                    |                               │
          |                                    └──> Lavapipe end-to-end ◄──────┘
          |                                                |
          +---> VirtIO GPU 3D ──┬──> DRM Device Node ──────┤
          |                     │                          │
          +---> DMA-buf ────────┘                          │
          |                                                │
          └──> Mesa virgl/venus ◄──────────────────────────┘
```

## Dependency Table

| Feature | Depends On | Blocks |
|---------|------------|--------|
| Input & Display Infrastructure | VFS, Userspace, Device Drivers (all complete) | VirtIO GPU, Compositor |
| VirtIO GPU 2D Driver | Device Drivers, Input & Display | Compositor, VirtIO GPU 3D |
| Compositor & 2D Graphics | IPC & Signals, Input & Display, VirtIO GPU | --- |
| Networking — TCP/UDP | VFS, Device Drivers (all complete) | --- |
| vDSO & Performance | Userspace (complete) | --- |
| sysfs | VFS, PCI enumeration (complete) | Mesa port |
| Unix Domain Sockets | VFS, IPC (complete) | Wayland subset |
| Wayland Minimal Subset | Unix Domain Sockets, Display Infrastructure | Mesa WSI, Lavapipe |
| Mesa & Vulkan (lavapipe) | sysfs, UDS, Wayland, mprotect, pthreads | Mesa Vulkan (GPU) |
| VirtIO GPU 3D | VirtIO GPU 2D | DRM device node |
| DRM Device Node | VirtIO GPU 3D, Dynamic devfs | Mesa virgl/venus |
| Mesa & Vulkan (virgl/venus) | DRM device node, DMA-buf, Mesa lavapipe | --- |

## Completed Features

| Feature | Status |
|---------|--------|
| Boot, PMM, VMM/Heap, Interrupts/APIC, Executor, Syscall Interface | Complete |
| Async VFS & Ramfs | Complete |
| Userspace & ELF Loading | Complete |
| Device Drivers | Complete |
| IPC & Minimal Signals | Complete |
| SMP & Per-CPU Executors | Complete |

## Deferred

| Item | Reason |
|------|--------|
| ext2 Filesystem | No immediate need for persistent on-disk FS |
| Real GPU (AMD/Intel) | Requires IOMMU, GEM/TTM, KMS — after VirtIO GPU Vulkan works |
| USB HID | Deferred until USB host controller work |

## Critical Path

The critical path to Vulkan on screen:

```
mprotect + sysfs + UDS + libc ──> Mesa port ──> Wayland ──> Lavapipe (Phase 1)
VirtIO GPU 3D + DRM node + DMA-buf ──> Mesa virgl/venus (Phase 2)
```

## Parallelization Opportunities

### Immediately Available

All prerequisites for these features are already complete:

- **sysfs** — depends on VFS + PCI (complete)
- **Unix Domain Sockets** — depends on VFS + IPC (complete)
- **mprotect** — depends on memory subsystem (complete)
- **libc additions** (poll, pthreads) — depends on existing syscalls (complete)
- **Networking** — builds on existing ARP/ICMP/IPv4 stack
- **vDSO** — all dependencies satisfied

sysfs, UDS, mprotect, and libc work can all proceed in parallel.

### After sysfs + UDS + libc

- **Mesa cross-compilation** — needs libc, sysfs
- **Wayland subset** — needs UDS

### After Mesa + Wayland

- **Lavapipe end-to-end** — software Vulkan triangle on screen (Phase 1 complete)

### After VirtIO GPU 2D

- **VirtIO GPU 3D** — extends existing 2D driver with 3D commands
- **DRM device node** — needs VirtIO GPU 3D + dynamic devfs

### After DRM + DMA-buf

- **Mesa virgl/venus** — hardware-accelerated Vulkan (Phase 2 complete)

## Recommended Order

For a single developer, the recommended sequential order:

### Near-term (remaining features)

1. **VirtIO GPU 2D** — proper display protocol (in progress)
2. **Networking** — TCP/UDP (extends existing stack)
3. **vDSO** — performance optimization

### Graphics stack (Phase 1 — Software Vulkan)

4. **mprotect** — unblocks shader JIT
5. **Dynamic devfs** — unblocks device node creation
6. **sysfs** — unblocks Mesa device discovery
7. **sys_query extensions** — replaces procfs for Mesa
8. **poll() + pthreads in libc** — unblocks Mesa compilation
9. **Unix Domain Sockets** — unblocks Wayland transport
10. **Mesa cross-compilation** — get it building against hadron sysroot
11. **Wayland minimal subset** — compositor + libwayland-client
12. **Lavapipe end-to-end test** — software Vulkan on screen

### Graphics stack (Phase 2 — GPU-Accelerated Vulkan)

13. **VirtIO GPU 3D commands** — extend existing driver
14. **DRM device node + ioctls** — userspace GPU access
15. **Minimal DMA-buf** — buffer sharing for compositor
16. **Mesa virgl/venus driver** — hardware-accelerated Vulkan
