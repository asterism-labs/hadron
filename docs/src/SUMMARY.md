# Summary

[Introduction](introduction.md)

# Architecture

- [Overview](architecture/overview.md)
- [Crate Structure](architecture/crate-structure.md)
- [Frame vs Services](architecture/frame-vs-services.md)

# Kernel Internals

- [Memory Management](internals/memory-management.md)
- [Async Executor](internals/executor.md)
- [Virtual Filesystem](internals/vfs.md)
- [Process Management](internals/process-management.md)
- [Syscall Interface](internals/syscalls.md)
- [Driver Model](internals/driver-model.md)
- [Synchronization Primitives](internals/synchronization.md)
- [Architecture & Boot](internals/arch-and-boot.md)
- [Hardware Drivers](internals/drivers.md)
- [Inter-Process Communication](internals/ipc.md)

# Completed Phases

- [Phase 8: Async VFS & Ramfs](phases/08-vfs-ramfs.md)
- [Phase 9: Userspace & ELF Loading](phases/09-userspace.md)
- [Phase 10: Device Drivers](phases/10-device-drivers.md)
- [Phase 11: IPC & Minimal Signals](phases/11-ipc-signals.md)
- [Phase 12: SMP & Per-CPU Executors](phases/12-smp.md)

# Remaining Phases

- [Phase 13: Input & Display Infrastructure](phases/13-input-display.md)
- [Phase 14: VirtIO GPU 2D Driver](phases/14-virtio-gpu.md)
- [Phase 15: Compositor & 2D Graphics](phases/15-compositor.md)
- [Phase 16: Networking — TCP/UDP](phases/16-networking.md)
- [Phase 17: vDSO & Performance](phases/17-vdso.md)

# Deferred

- [ext2 Filesystem](phases/deferred-ext2.md)

# Design Decisions

- [Task-Centric OS Design](design/task-centric-design.md)
- [Executor Architecture](design/executor-architecture.md)
- [Preemption & Scaling](design/preemption-and-scaling.md)
- [Syscall Strategy](design/syscall-strategy.md)
- [POSIX Compatibility](design/posix-compatibility.md)
- [Kernel Architecture Comparison](design/kernel-comparison.md)
- [Memory Layout](design/memory-layout.md)
- [Boot Procedure](design/boot-procedure.md)

# Reference

- [Build System](reference/build-system.md)
- [Target File Tree](reference/file-tree.md)
- [Phase Dependencies](reference/phase-dependencies.md)
- [Estimated Scope](reference/estimated-scope.md)
- [Known Issues](reference/known-issues.md)
