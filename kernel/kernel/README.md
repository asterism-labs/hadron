# hadron-kernel

The monolithic core of the Hadron kernel. This crate contains architecture-specific primitives, the driver API trait hierarchy, memory management glue, the scheduler integration layer, VFS wiring, PCI bus infrastructure, process management, and the syscall dispatch table. It is the central crate that ties together the independent `hadron-mm`, `hadron-sched`, and `hadron-fs` subsystem crates with arch-specific code and the boot entry point. All pluggable hardware drivers in `hadron-drivers` depend on the trait interfaces defined here.

## Features

- **Bootloader-agnostic entry point** -- a `BootInfo` trait abstracts over Limine, UEFI, and other boot protocols; the single `kernel_init` function handles all post-boot initialization regardless of the stub that called it
- **x86_64 architecture support** -- GDT/TSS, IDT with per-vector handlers, APIC (Local + I/O), HPET/PIT/TSC timers, 4-level and 5-level paging with per-section permissions (NX, WP, Global), `SYSCALL`/`SYSRET` fast path, and SMP bootstrap with per-CPU GS-based state
- **Layered driver API** -- a four-layer trait model (resources, base `Driver`, category traits like `PlatformDriver`, and interface traits like `SerialPort`, `Framebuffer`, `BlockDevice`, `KeyboardDevice`) with typed capability tokens (`IrqCapability`, `MmioCapability`, `DmaCapability`, `PciConfigCapability`) and probe contexts that bundle them for safe driver initialization
- **Linker-section driver registration** -- PCI and platform drivers are placed into dedicated linker sections at compile time and matched against discovered devices at boot, with no runtime registration API
- **Device registry** -- a central registry where probed drivers deposit their device objects (block devices, framebuffers, keyboards); the kernel retrieves them by name for VFS mounting, display output, and input handling
- **Memory management glue** -- wraps the `hadron-mm` crate's PMM, VMM, and heap with boot-info conversion, HHDM initialization, and the `#[global_allocator]` setup; provides a guarded kernel stack allocator and MMIO mapping helpers
- **Async cooperative scheduler** -- integrates the `hadron-sched` executor with architecture-specific `sti; hlt` idle, timer-tick preemption, cross-CPU IPI wakeup, and work stealing across per-CPU executor instances
- **Process management** -- ELF loading via `hadron-elf`, per-process address spaces with independent page tables, ring-0/ring-3 transitions via `iretq` and `SYSRET`, timer-driven preemption with saved register snapshots, `TRAP_WAIT` and `TRAP_IO` for async blocking syscalls, signal delivery, and a global process table with waitpid reaping
- **Syscall dispatch** -- routes syscall numbers through a generated `SyscallHandler` trait to handlers for process lifecycle (`task_exit`, `task_spawn`, `task_wait`, `task_kill`), VFS operations (`vnode_open`, `vnode_read`, `vnode_write`, `vnode_stat`, `vnode_readdir`), memory mapping (`mem_map`, `mem_unmap`), pipes, handle management, time queries, and debug logging
- **PCI bus core** -- legacy CAM and ECAM (MMIO) configuration space access, recursive bus enumeration, and capability parsing (MSI, MSI-X, power management)
- **VFS integration** -- mounts ramfs as root, unpacks CPIO initrd archives, mounts devfs at `/dev`, and discovers block-device-backed filesystems (FAT, ISO 9660) from the driver registry
- **IPC** -- kernel-level pipe implementation for inter-process communication
- **Framebuffer console (fbcon)** -- cell-based framebuffer rendering with ANSI escape code support, used as a log sink after driver probing
- **Profiling** -- optional sampling profiler and ftrace instrumentation, gated behind build-time configuration flags
- **Backtrace support** -- symbolicated kernel backtraces using embedded HKIF debug data
- **Multi-sink logging** -- serial, early framebuffer, and fbcon sinks with configurable log levels (`Error`, `Warn`, `Info`, `Debug`, `Trace`) and per-subsystem trace filtering

## Architecture

The crate is organized into the following top-level modules:

- **`arch`** -- architecture-specific code behind `cfg(target_arch)` gates. The `x86_64` subtree contains GDT/IDT setup, APIC/HPET/PIT/TSC drivers, paging structures, register access, interrupt dispatch, SMP bootstrap, syscall entry stubs, and userspace transition routines. A stub `aarch64` module provides the trait signatures for future porting.
- **`boot`** -- the `BootInfo` trait, `BootInfoData` container, and `kernel_init` entry point that orchestrates the full initialization sequence from CPU init through executor launch.
- **`driver_api`** -- the four-layer driver model: `resource` (IoPortRange, MmioRegion, IrqLine), `driver` (Driver trait, DriverInfo), `category` (PlatformDriver), interface traits (`serial`, `block`, `framebuffer`, `input`, `hw`), `capability` tokens, `probe_context` bundles, `registration` linker-section entry types, and `lifecycle` (ManagedDriver for suspend/resume).
- **`drivers`** -- kernel-internal driver infrastructure: device registry, early serial/framebuffer console, IRQ management, fbcon renderer, and the linker-section registry reader.
- **`mm`** -- memory management glue that wraps `hadron-mm` with boot-info conversion and arch-specific TLB flush registration; provides the `init` sequence for PMM, VMM, and heap.
- **`sched`** -- scheduler glue that wraps `hadron-sched` with arch-specific halt, SMP work stealing, and `block_on` for synchronous contexts.
- **`pci`** -- PCI bus core: CAM/ECAM configuration access, bus enumeration, and capability parsing.
- **`proc`** -- process management: ELF binary loading (`binfmt`), process creation (`exec`), the async `process_task` loop, signal handling, and the global process table.
- **`syscall`** -- syscall dispatch and handler modules for I/O, memory, process, VFS, time, and query operations, plus `userptr` validation.
- **`fs`** -- VFS wiring: mount table integration, devfs construction, block-device adapter, and console input device.
- **`ipc`** -- inter-process communication primitives (pipes).
- **`bus`** -- bus abstraction layer for device enumeration.
- **`log`** -- multi-sink kernel logger with serial, framebuffer, and fbcon backends.
- **`time`** -- monotonic boot clock (`boot_nanos`, `timer_ticks`) backed by HPET or TSC.
- **`profiling`** -- optional sampling and ftrace profiling infrastructure.
- **`backtrace`** -- kernel stack trace symbolication from embedded HKIF data.
- **`percpu`** -- per-CPU state structure and GS-base accessor helpers.
- **`config`** -- build-time configuration constants resolved by gluon.
