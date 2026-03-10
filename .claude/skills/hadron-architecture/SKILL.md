---
name: hadron-architecture
description: Use when exploring kernel architecture, boot flow, crate dependencies, custom target, or writing/running tests
---

# Hadron Architecture & Testing

## Architecture Model

Hadron is a capability-based object microkernel inspired by Zircon. All kernel resources are typed objects implementing the `KernelObject` trait, identified by unique `Koid`s. Userspace accesses objects through capability handles with explicit `Rights` bitflags. Hardware drivers run in userspace (ring 3) as isolated processes.

## Boot Flow

```
hadron-boot-uefi (PE32+ EFI app, kernel/boot/uefi)
  → Load kernel ELF, build HHDM page tables, ExitBootServices
  → kernel_init(&BootInfo)
    → GDT/IDT/TSS, PMM, VMM, heap
    → ACPI, LAPIC/IO-APIC, SMP (INIT-SIPI-SIPI)
    → IOMMU, PCI enumeration
    → Per-CPU async executor
    → (Future: root task, userboot, devmgr)
```

## Object Model

The kernel object system (`kernel/objects`) implements typed kernel objects:

**Implemented:** Process, Thread, Vmo (Paged/Cow/Pager/Contiguous), Vmar, HandleTable

**Planned (Phase 2):** Channel, Socket, Port, Event, EventPair, Timer, Fifo, Resource, Job, Interrupt, Iommu, Bti, Pmt

Core concepts:
- `KernelObject` trait — common interface for all object types
- `Koid` — kernel object ID, globally unique
- `Signals` — per-object signal bitflags for async notification
- `HandleEntry` / `HandleTable` — per-process handle-to-object mapping
- `Rights` — per-handle access control bitflags

## Key Crates

### kernel/
- `core` — hadron-core: addr types, sync primitives, lockdep, cpu_local, paging
- `objects` — hadron-objects: KernelObject trait, Handle, Rights, Process, Thread, Vmo, Vmar
- `mm` — hadron-mm: PMM, page table mapper, VMO/VMAR, AddressSpace, heap
- `sched` — hadron-sched: per-CPU async executor, timer wheel, work stealing
- `pci` — hadron-pci: PCI enumeration, capability parsing, VirtIO transport
- `mmio` — hadron-mmio: typed MMIO register block macros
- `mmio-macros` — proc macro crate for `register_block!`
- `intrinsics` — SIMD/SSE2/AVX intrinsic wrappers
- `kernel` — hadron-kernel: main kernel binary, arch code, boot entry, SMP
- `kernel-image` — kernel image packaging
- `boot/uefi` — hadron-boot-uefi: UEFI boot stub (PE32+ EFI application)

### crates/
- `core/linkset` — linker-section data access macros
- `boot/uefi` — UEFI type bindings and boot protocol definitions
- `parse/acpi` — ACPI table parser (RSDP, MADT, DMAR, MCFG, etc.)
- `parse/elf` — ELF64 parser
- `parse/binparse` + `binparse-macros` — binary structure parsing framework
- `parse/dwarf` — DWARF debug info parser
- `parse/fdt` — Flattened Device Tree parser

### tools/
- `hadron-log` — kernel logging infrastructure
- `hadron-boot-info` — boot information structures shared between boot stub and kernel
- `hadron-ktest` — kernel integration test framework
- `hadron-bench` — kernel benchmark framework
- `hadron-test` — test runner and QEMU integration
- `hadron-utest` — userspace test utilities
- `gluon` — custom Rhai-scripted build system
- `hadron-perf` — performance measurement
- `hadron-runner` — test runner

### userspace/
- `lepton-syslib` — native Rust syscall wrapper library (primary userspace API)
- `hadron-libc` — C runtime compatibility types

**Note:** `kernel/syscall/` and `kernel/syscall-macros/` are documented in the architecture but not yet created. No `hadron-drivers` crate exists — drivers will run in userspace.

## Custom Target

The kernel uses a custom target `x86_64-unknown-hadron` (not `x86_64-unknown-none`):
- Kernel code model, PIC relocation
- Soft-float (no SSE/AVX in kernel mode)
- Panic = abort, redzone disabled
- Uses `rust-lld` linker

## Userspace Architecture

- **lepton-syslib**: Native syscall wrapper library — the primary API for new userspace code
- **hadron-libc**: POSIX compatibility shim translating libc calls to native Hadron syscalls. Secondary layer for porting existing POSIX applications
- New userspace programs should use `lepton-syslib` directly; `hadron-libc` exists for compatibility, not as the project's primary interface

## Testing

- `gluon test --host-only` — Run host unit tests for crates listed in `gluon.rhai` `tests().host_testable()`
- `gluon test --kernel-only` — Build kernel + run integration tests in QEMU
- `gluon test` — Run both host and kernel tests

Integration tests run in QEMU using `hadron-test` crate:
- Tests use `isa-debug-exit` device (iobase=0xf4) to signal pass/fail
- Exit code 33 = success (configured in `gluon.rhai` `qemu()` section)
- Timeout: 30 seconds per test
