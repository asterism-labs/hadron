# Summary

[Introduction](introduction.md)

# Architecture & Concepts

- [Overview](architecture/overview.md)
- [Crate Structure](architecture/crate-structure.md)
- [Frame vs Services](architecture/frame-vs-services.md)

# Architecture & Implementation

- [Task Execution & Scheduling](architecture/task-execution.md)
- [Memory & Allocation](architecture/memory.md)
- [I/O & Filesystem](architecture/io-filesystem.md)
- [Synchronization & IPC](architecture/sync-ipc.md)
- [Architecture & Boot](internals/arch-and-boot.md)
- [Driver Architecture](internals/driver-model.md)
- [Hardware Drivers](internals/drivers.md)

# Completed Features

- [Async VFS & Ramfs](features/vfs-ramfs.md)
- [Userspace & ELF Loading](features/userspace.md)
- [Device Drivers](features/device-drivers.md)
- [IPC Channels & Shared Memory](features/ipc-channels.md)
- [TTY & Terminal System](features/tty-system.md)
- [Display Infrastructure](features/display-infrastructure.md)
- [Input Handling](features/input-handling.md)
- [IPC & Signal Handling](features/ipc-signals.md)
- [Threading & task_clone](features/threading.md)
- [SMP & Per-CPU Executors](features/smp.md)
- [Network Stack - Phase 1 (ARP & ICMP)](features/network-phase1.md)
- [Userspace Compositor](features/compositor.md)

# Remaining Features

- [Network Stack - Phase 2 (TCP/UDP)](features/networking.md)
- [vDSO & Performance](features/vdso.md)
- [VirtIO GPU 2D Driver](features/virtio-gpu.md)

# Graphics Stack

- [sysfs Virtual Filesystem](features/sysfs.md)
- [Unix Domain Sockets](features/unix-domain-sockets.md)
- [Mesa & Vulkan](features/mesa-vulkan.md)
- [Wayland Minimal Subset](features/wayland-wsi.md)

# Deferred

- [ext2 Filesystem](features/deferred-ext2.md)

# Design Decisions

- [Task-Centric OS Design](design/task-centric-design.md)
- [Executor Architecture](design/executor-architecture.md)
- [Preemption & Scaling](design/preemption-and-scaling.md)
- [Syscall Strategy](design/syscall-strategy.md)
- [POSIX Compatibility](design/posix-compatibility.md)
- [Kernel Architecture Comparison](design/kernel-comparison.md)
- [Memory Layout](design/memory-layout.md)
- [Boot Procedure](design/boot-procedure.md)
- [Graphics Stack](design/graphics-stack.md)

# Reference

- [Build System](reference/build-system.md)
- [Target File Tree](reference/file-tree.md)
- [Feature Dependencies](reference/feature-dependencies.md)
- [Estimated Scope](reference/estimated-scope.md)
- [Known Issues](reference/known-issues.md)
