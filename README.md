# Hadron

Capability-based microkernel for x86_64, written in Rust.

Hadron is a Zircon-inspired microkernel built around a kernel object model with capability-based access control. The ring-0 kernel provides object lifecycle management, IPC, memory management, and scheduling. Hardware drivers, filesystems, and networking run as isolated userspace (ring 3) processes that receive capability handles to access resources.

## Architecture

- **Object model**: All kernel resources (processes, threads, VMOs, channels, etc.) are typed objects implementing the `KernelObject` trait, identified by unique `Koid`s
- **Capability handles**: Userspace accesses objects through handles with associated `Rights` bitflags — no ambient authority
- **Userspace drivers**: Hardware drivers run in isolated `driver-host` processes, receiving MMIO VMO, Interrupt, and Bti handles via their initial handle table

## Crate Layout

```
hadron/
├── kernel/                 # Ring-0 kernel crates
│   ├── core/               # Address types, sync primitives, lockdep, cpu_local
│   ├── objects/            # Kernel object system (KernelObject, Handle, Rights)
│   ├── mm/                 # PMM, page table mapper, VMO, VMAR, AddressSpace
│   ├── sched/              # Per-CPU async executor and task scheduler
│   ├── pci/                # PCI bus enumeration and capability parsing
│   ├── mmio/               # Typed MMIO register block macros
│   ├── intrinsics/         # SIMD/SSE2/AVX intrinsic wrappers
│   ├── kernel/             # Main kernel binary (hadron-kernel)
│   ├── kernel-image/       # Kernel image packaging
│   └── boot/uefi/          # UEFI boot stub (PE32+ EFI application)
├── crates/                 # Host-testable libraries
│   ├── boot/uefi/          # UEFI type bindings
│   ├── core/linkset/       # Linker-section data access macros
│   └── parse/              # acpi, elf, binparse, dwarf, fdt
├── userspace/              # Ring-3 programs and libraries
│   ├── lepton-syslib/      # Native Rust syscall wrapper library
│   └── hadron-libc/        # C runtime compatibility types
├── tools/                  # Build and test tooling
│   ├── gluon/              # Custom Rhai-scripted build system
│   ├── hadron-log/         # Kernel logging infrastructure
│   ├── hadron-perf/        # Performance measurement
│   └── hadron-runner/      # Test runner
└── docs/                   # mdbook documentation
```

## Quick Start

```sh
just vendor              # Fetch vendored dependencies
just configure           # Resolve config + generate rust-project.json
just build               # Build kernel + all crates
just run                 # Build + run in QEMU
just test                # Run all tests (host + kernel)
just test --host-only    # Host-side unit tests only (fast)
just test --kernel-only  # Kernel integration tests only (QEMU)
just check               # Type-check without linking
just clippy              # Run clippy lints
just fmt                 # Format source files
just script <file.rhai>  # Run Rhai script against booted QEMU
just script              # Interactive REPL for QEMU scripting
```

A `justfile` at the project root wraps `gluon` (auto-bootstrapped on first run). Build configuration lives in `gluon.rhai` (Rhai scripting).

## Documentation

See [`docs/`](docs/) for full documentation, including architecture details, crate descriptions, and development guides.
