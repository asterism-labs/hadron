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
```

## Dependency Table

| Feature | Depends On | Blocks |
|---------|------------|--------|
| Input & Display Infrastructure | VFS, Userspace, Device Drivers (all complete) | VirtIO GPU, Compositor |
| VirtIO GPU 2D Driver | Device Drivers, Input & Display | Compositor |
| Compositor & 2D Graphics | IPC & Signals, Input & Display, VirtIO GPU | --- |
| Networking — TCP/UDP | VFS, Device Drivers (all complete) | --- |
| vDSO & Performance | Userspace (complete) | --- |

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
| OpenGL/Vulkan | Requires Mesa port, long-term aspiration |
| USB HID | Deferred until USB host controller work |

## Critical Path

The critical path to the graphical compositor:

```
Input & Display --> VirtIO GPU --> Compositor
```

## Parallelization Opportunities

### Immediately Available

All prerequisites for these features are already complete:

- **Input & Display** — all dependencies satisfied
- **Networking** — builds on existing ARP/ICMP/IPv4 stack
- **vDSO** — all dependencies satisfied

Input & Display, Networking, and vDSO can proceed in parallel.

### After Input & Display

- **VirtIO GPU** — needs Input & Display for devfs framebuffer integration

### After VirtIO GPU

- **Compositor** — needs VirtIO GPU (though can start with Bochs VGA)

## Recommended Order

For a single developer, the recommended sequential order:

1. **Input & Display** — input & display infrastructure (enables graphical userspace)
2. **VirtIO GPU** — VirtIO GPU 2D (proper display protocol)
3. **Compositor** — compositor & 2D graphics (graphical desktop)
4. **Networking** — TCP/UDP (extends existing stack)
5. **vDSO** — vDSO & performance (optimization, lowest priority)
