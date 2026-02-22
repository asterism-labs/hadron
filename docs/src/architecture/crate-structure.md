# Crate Structure

Hadron follows a two-crate kernel model with standalone library crates and a custom build system.

## Project Layout

```
hadron/
├── gluon.rhai              # Build configuration (Rhai scripting)
├── justfile                # Primary build interface
│
├── kernel/
│   ├── hadron-kernel/      # Monolithic kernel: arch, driver API, mm, sched, VFS, PCI
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── addr.rs         # PhysAddr, VirtAddr, PhysFrame newtypes
│   │       ├── boot.rs         # BootInfo trait, kernel_init
│   │       ├── paging.rs       # High-level paging abstractions
│   │       ├── percpu.rs       # Per-CPU data, CpuLocal<T>
│   │       ├── task.rs         # TaskId, task types
│   │       ├── arch/x86_64/    # GDT, IDT, ACPI, SMP, interrupts, instructions, registers
│   │       ├── mm/             # PMM, VMM, heap, HHDM, address_space, region, zone
│   │       ├── sync/           # SpinLock, IrqSpinLock, Mutex, RwLock, WaitQueue, Lazy
│   │       ├── sched/          # Async executor, waker encoding, timer, SMP scheduling
│   │       ├── fs/             # VFS, devfs, console_input, block_adapter, file, path
│   │       ├── proc/           # Process management, ELF loading (binfmt)
│   │       ├── syscall/        # Syscall dispatch, io, memory, process, query, time, vfs
│   │       ├── ipc/            # Pipes, IPC primitives
│   │       ├── driver_api/     # Driver trait hierarchy (resources, categories, interfaces)
│   │       ├── drivers/        # Device registry
│   │       └── pci/            # PCI enumeration, BAR decoding, capabilities
│   │
│   ├── hadron-drivers/     # Pluggable hardware drivers
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ahci/           # AHCI (SATA) driver
│   │       ├── virtio/         # VirtIO block, PCI transport, virtqueues
│   │       ├── serial/         # UART16550, async serial
│   │       ├── input/          # i8042, keyboard, mouse
│   │       ├── display/        # Bochs VGA
│   │       ├── block/          # Ramdisk
│   │       └── fs/             # FAT, ISO9660, ramfs, initramfs
│   │
│   └── boot/
│       └── limine/         # Limine boot stub binary (hadron-boot-limine)
│
├── crates/
│   ├── parse/
│   │   ├── acpi/                     # ACPI table parsing (hadron-acpi)
│   │   ├── binparse/                 # Binary format parser (hadron-binparse)
│   │   ├── binparse-macros/           # Companion proc-macro
│   │   ├── dwarf/                    # DWARF debug info (hadron-dwarf)
│   │   └── elf/                      # ELF64 parser (hadron-elf)
│   ├── boot/
│   │   ├── limine/                   # Limine boot protocol bindings
│   │   └── uefi/                     # UEFI bindings
│   ├── core/
│   │   ├── hadron-core/              # Core kernel abstractions
│   │   └── linkset/                  # Linker-section set collections (hadron-linkset)
│   ├── driver/
│   │   ├── hadron-driver-macros/     # #[hadron_driver] proc-macro
│   │   ├── hadron-mmio/              # MMIO register abstraction
│   │   └── hadron-mmio-macros/       # Companion proc-macro
│   ├── syscall/
│   │   ├── hadron-syscall/           # Syscall numbers and ABI definitions
│   │   └── hadron-syscall-macros/    # Companion proc-macro
│   ├── test/
│   │   ├── hadron-test/              # Test framework (QEMU isa-debug-exit)
│   │   └── hadron-bench/             # Benchmark framework
│   └── tools/
│       ├── hadron-codegen/           # Code generation utilities
│       └── hadron-perf/              # Performance analysis tools
│
├── userspace/
│   ├── init/               # Init process (lepton-init)
│   ├── lepton-syslib/      # Userspace syscall library
│   ├── shell/              # Interactive shell (lepton-shell)
│   └── coreutils/          # Core utilities (lepton-coreutils)
│
├── tools/
│   └── gluon/              # Custom build system
│
├── vendor/                 # Vendored external dependencies
├── targets/                # Custom target specs (x86_64-unknown-hadron.json)
└── docs/                   # This mdbook
```

## Crate Dependency Graph

```
kernel/boot/limine ──┬──> kernel/hadron-kernel
                     ├──> kernel/hadron-drivers
                     ├──> crates/boot/limine
                     └──> planck-noalloc (external)

kernel/hadron-drivers ──┬──> kernel/hadron-kernel
                        ├──> bitflags
                        ├──> hadris-cpio, hadris-fat, hadris-io, hadris-iso
                        └──> (registers via linker-section macros)

kernel/hadron-kernel ──┬──> bitflags
                       ├──> hadris-io
                       ├──> hadron-acpi, hadron-elf, hadron-syscall
                       └──> planck-noalloc (external)

crates/boot/limine      ──> (no deps)
crates/test/hadron-test ──> (no deps)
```

Key design principle: **`crates/*/*` are standalone, no_std libraries** that can be tested independently and reused in other projects.

## Crate Responsibilities

### `kernel/hadron-kernel` — Monolithic Kernel

Contains both the unsafe frame (arch-specific, hardware-touching code) and safe services (subsystems built on safe abstractions). The framekernel safety boundary is enforced at the module level within this crate.

**Frame modules** (contain `unsafe`):

| Module | Purpose |
|--------|---------|
| `addr.rs` | `PhysAddr`, `VirtAddr`, `PhysFrame` newtypes |
| `paging.rs` | Page table types and mapping abstractions |
| `arch/x86_64/` | GDT, IDT, ACPI, SMP, instructions, registers, interrupts, syscall entry |
| `mm/` | HHDM, PMM (bitmap allocator), VMM, heap, address spaces, regions, zones |
| `sync/` | `SpinLock`, `IrqSpinLock`, `Mutex`, `RwLock`, `WaitQueue`, `Lazy` |

**Service modules** (safe Rust):

| Module | Purpose |
|--------|---------|
| `boot.rs` | `BootInfo` trait, `kernel_init` entry point |
| `sched/` | Async executor, priority tiers, waker encoding, timer integration |
| `fs/` | VFS mount table, devfs, console input, file descriptors, path resolution |
| `proc/` | Process struct, ELF loading (binfmt), userspace entry/exit |
| `syscall/` | Syscall dispatch table, I/O, memory, process, time categories |
| `ipc/` | Pipes (circular buffer, Inode implementation) |
| `driver_api/` | Driver trait hierarchy (resources, categories, interfaces) |
| `drivers/` | Device registry |
| `pci/` | PCI enumeration, BAR decoding, capabilities parsing |

### `kernel/hadron-drivers` — Pluggable Drivers

Hardware driver implementations registered via linker-section macros (`pci_driver_entry!`, `platform_driver_entry!`, `block_fs_entry!`). Depends on `hadron-kernel` for the driver API traits.

| Module | Purpose |
|--------|---------|
| `ahci/` | AHCI (SATA) controller driver |
| `virtio/` | VirtIO block device, PCI transport, virtqueues |
| `serial/` | UART16550 driver, async serial interface |
| `input/` | i8042 controller, PS/2 keyboard and mouse |
| `display/` | Bochs VGA display driver |
| `block/` | Ramdisk block device |
| `fs/` | FAT, ISO9660, ramfs, initramfs filesystem implementations |

### `crates/*/*` — Reusable Libraries

| Crate | Group | Purpose |
|-------|-------|---------|
| `hadron-acpi` | parse | ACPI table parsing (RSDP, MADT, HPET, FADT) |
| `hadron-binparse` | parse | Binary format parser with derive macro |
| `hadron-dwarf` | parse | DWARF debug info parsing |
| `hadron-elf` | parse | ELF64 parser (program headers, sections, entry point) |
| `limine` | boot | Limine boot protocol bindings |
| `uefi` | boot | UEFI bindings |
| `hadron-core` | core | Core kernel abstractions (sync primitives, wait queues) |
| `hadron-linkset` | core | Linker-section set collections |
| `hadron-mmio` | driver | MMIO register abstraction with derive macro |
| `hadron-driver-macros` | driver | `#[hadron_driver]` proc-macro |
| `hadron-syscall` | syscall | Syscall numbers and ABI definitions (shared kernel/userspace) |
| `hadron-test` | test | Test framework (QEMU isa-debug-exit, test runner) |
| `hadron-bench` | test | Benchmark framework |

**External dependency**: `planck-noalloc` — allocation-free data structures (ring buffer, array vec) — published on crates.io, vendored via gluon.

### `kernel/boot/*` — Boot Stubs

Binary crates that bridge bootloader protocols to the kernel.

| Stub | Crate Name | Bootloader | Purpose |
|------|-----------|-----------|---------|
| `kernel/boot/limine/` | `hadron-boot-limine` | Limine | Primary boot path, HHDM provided by bootloader |
