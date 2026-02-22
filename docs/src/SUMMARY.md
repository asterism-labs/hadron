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

# Development Phases

- [Phase 8: Async VFS & Ramfs](phases/08-vfs-ramfs.md)
- [Phase 9: Userspace & ELF Loading](phases/09-userspace.md)
- [Phase 10: Device Drivers](phases/10-device-drivers.md)
- [Phase 11: IPC & Minimal Signals](phases/11-ipc-signals.md)
- [Phase 12: SMP & Per-CPU Executors](phases/12-smp.md)
- [Phase 13: ext2 Filesystem](phases/13-ext2.md)
- [Phase 14: Networking](phases/14-networking.md)
- [Phase 15: vDSO & Performance](phases/15-vdso.md)

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
