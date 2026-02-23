# Phase Dependencies

This chapter shows how the remaining development phases (13-17) depend on each other. Phases 0-12 are complete.

## Dependency Graph

```
Completed (Phases 0-12)
    |
    +---> Phase 13: Input & Display ---> Phase 14: VirtIO GPU ---> Phase 15: Compositor
    |                                         |
    |                                         +--- (Phase 15 also needs Phase 11: IPC)
    |
    +---> Phase 16: Networking (builds on existing ARP/ICMP/IPv4)
    |
    +---> Phase 17: vDSO (needs Phase 9: userspace)
```

## Dependency Table

| Phase | Name | Depends On | Blocks |
|-------|------|------------|--------|
| 13 | Input & Display Infrastructure | 8, 9, 10 (all complete) | 14, 15 |
| 14 | VirtIO GPU 2D Driver | 10, 13 | 15 |
| 15 | Compositor & 2D Graphics | 11, 13, 14 | --- |
| 16 | Networking — TCP/UDP | 8, 10 (all complete) | --- |
| 17 | vDSO & Performance | 9 (complete) | --- |

## Completed Phases

| Phase | Name | Status |
|-------|------|--------|
| 0-7 | Boot through syscalls | Complete |
| 8 | Async VFS & Ramfs | Complete |
| 9 | Userspace & ELF Loading | Complete |
| 10 | Device Drivers | Complete |
| 11 | IPC & Minimal Signals | Complete |
| 12 | SMP & Per-CPU Executors | Complete |

## Deferred

| Item | Original Phase | Reason |
|------|---------------|--------|
| ext2 Filesystem | 13 | No immediate need for persistent on-disk FS |
| OpenGL/Vulkan | --- | Requires Mesa port, long-term aspiration |
| USB HID | --- | Deferred until USB host controller work |

## Critical Path

The critical path to the graphical compositor:

```
Phase 13 (Input & Display) --> Phase 14 (VirtIO GPU) --> Phase 15 (Compositor)
```

## Parallelization Opportunities

### Immediately Available

All prerequisites for these phases are already complete:

- **Phase 13** (Input & Display) — all dependencies satisfied
- **Phase 16** (Networking) — builds on existing ARP/ICMP/IPv4 stack
- **Phase 17** (vDSO) — all dependencies satisfied

Phases 13, 16, and 17 can proceed in parallel.

### After Phase 13

- **Phase 14** (VirtIO GPU) — needs Phase 13 for devfs framebuffer integration

### After Phase 14

- **Phase 15** (Compositor) — needs Phase 14 for VirtIO GPU (though can start with Bochs VGA)

## Recommended Order

For a single developer, the recommended sequential order:

1. **Phase 13** — input & display infrastructure (enables graphical userspace)
2. **Phase 14** — VirtIO GPU 2D (proper display protocol)
3. **Phase 15** — compositor & 2D graphics (graphical desktop)
4. **Phase 16** — networking TCP/UDP (extends existing stack)
5. **Phase 17** — vDSO & performance (optimization, lowest priority)
