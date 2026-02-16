# Phase Dependencies

This chapter shows how the remaining development phases (7-15) depend on each other. Phases 0-6 are complete and documented in [Completed Work](../phases/completed-work.md).

## Dependency Graph

```
Completed (Phases 0-6)
    |
Phase 7: Syscall Interface [IN PROGRESS]
    |
    +---> Phase 8: Async VFS ---> Phase 9: Userspace ---> Phase 11: IPC & Signals
    |         |                                                |
    |         +---> Phase 13: ext2 <--- Phase 10: Drivers -----+
    |
    +---> Phase 12: SMP (parallel with 8-10, no dependency on userspace)
    |
    +---> Phase 14: Networking (needs 8 + 10)
    |
    +---> Phase 15: vDSO (needs 9)
```

## Dependency Table

| Phase | Name | Depends On | Blocks |
|-------|------|------------|--------|
| 7 | Syscall Interface | Completed (0-6) | 8, 12, 14, 15 |
| 8 | Async VFS & Ramfs | 7 | 9, 13, 14 |
| 9 | Userspace & ELF Loading | 7, 8 | 11, 15 |
| 10 | Device Drivers | 7 | 13, 14 |
| 11 | IPC & Minimal Signals | 8, 9 | --- |
| 12 | SMP & Per-CPU Executors | 7 | --- |
| 13 | ext2 Filesystem | 8, 10 | --- |
| 14 | Networking | 8, 10 | --- |
| 15 | vDSO & Performance | 7, 9 | --- |

## Critical Path

The critical path to the first userspace program:

```
Phase 7 (Syscalls) --> Phase 8 (VFS) --> Phase 9 (Userspace)
```

Three phases of sequential work from current state to first user program.

## Parallelization Opportunities

### After Phase 7 (Syscalls)

These can proceed independently:
- **Phase 8** (VFS) --- continue toward userspace
- **Phase 10** (Drivers) --- PCI, VirtIO
- **Phase 12** (SMP) --- can start as soon as syscalls work

### After Phase 9 (Userspace)

These are independent of each other:
- **Phase 11** (IPC) --- pipes, signals, sys_spawn
- **Phase 15** (vDSO) --- performance optimization

### Key Insight

Phase 12 (SMP) has no dependency on userspace and can be developed in parallel with Phases 8-10. Getting SMP online early catches concurrency bugs in all subsequent phases.

Phase 13 (ext2) depends on both Phase 8 (VFS trait) and Phase 10 (block device drivers). Phase 14 (Networking) similarly needs VFS (for socket FDs) and drivers (VirtIO-net).

## Recommended Order

For a single developer, the recommended sequential order:

1. **Phase 7** --- syscall interface (in progress)
2. **Phase 8** --- async VFS and ramfs
3. **Phase 9** --- userspace and ELF loading (critical path to first user program)
4. **Phase 10** --- device drivers (PCI, VirtIO-blk)
5. **Phase 11** --- IPC and signals (enables multi-process userspace)
6. **Phase 12** --- SMP (can also be done earlier, after Phase 7)
7. **Phase 13** --- ext2 filesystem
8. **Phase 14** --- networking
9. **Phase 15** --- vDSO and performance (lowest priority)
