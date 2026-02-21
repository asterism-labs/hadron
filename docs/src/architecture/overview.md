# Architecture Overview

Hadron follows the **framekernel** architecture, a design that combines the performance characteristics of a monolithic kernel with the safety guarantees enabled by Rust's type system.

## What is a Framekernel?

A framekernel splits the kernel into two layers:

1. **The Frame** — A minimal unsafe core that directly interacts with hardware. This is the only code that uses `unsafe` Rust. It provides safe abstractions over hardware primitives.
2. **Safe Services** — The rest of the kernel, written entirely in safe Rust. Services implement high-level functionality (scheduler, filesystem, drivers) using the safe APIs exported by the frame.

This is conceptually similar to a microkernel's separation of concerns, but without the IPC overhead — both layers run in the same address space (ring 0) and communicate through direct function calls.

## Layer Architecture

| Layer | Location | Safety | Responsibility |
|-------|----------|--------|----------------|
| **Frame** | `hadron-kernel::arch`, `mm`, `sync` | Contains `unsafe` | Physical memory, page tables, interrupts, CPU state, I/O ports |
| **Safe Services** | `hadron-kernel::fs`, `sched`, `syscall`, `proc`, etc. | Safe Rust only | Executor, VFS, syscall dispatch, process management |
| **Drivers** | `hadron-drivers` | Safe Rust (uses driver API traits) | AHCI, VirtIO, serial, display, input, filesystem implementations |
| **Boot Stubs** | `kernel/boot/{limine}/` | `unsafe` (entry) | Bootloader-specific entry, translates to `BootInfo` |
| **Reusable Crates** | `crates/{limine,noalloc,hadron-test,acpi,elf,...}/` | Varies | Standalone no_std libraries and protocol bindings |

## Safety Model

The framekernel's key insight is using Rust's module visibility and type system as the safety boundary:

```
┌─────────────────────────────────────────────────────┐
│                    hadron-kernel                     │
│                                                     │
│  ┌─────────┐ ┌──────┐ ┌─────────┐ ┌───────────┐   │
│  │Executor │ │ VFS  │ │Syscalls │ │  Proc Mgr │   │
│  └────┬────┘ └──┬───┘ └────┬────┘ └─────┬─────┘   │
│       │         │          │             │          │
│  ─────┴─────────┴──────────┴─────────────┴────────  │
│         Safe API boundary (module visibility)        │
│  ─────────────────────────────────────────────────  │
│          arch/, mm/, sync/ (unsafe frame)            │
│                                                     │
│  ┌──────┐ ┌───────────┐ ┌─────┐ ┌──────────────┐  │
│  │ PMM  │ │Page Tables│ │ IDT │ │    SYSCALL   │  │
│  └──────┘ └───────────┘ └─────┘ └──────────────┘  │
│                                                     │
│  ┌──────┐ ┌──────┐ ┌─────────┐ ┌─────────┐       │
│  │ GDT  │ │ APIC │ │ I/O Ops │ │SpinLock │       │
│  └──────┘ └──────┘ └─────────┘ └─────────┘       │
└─────────────────────────────────────────────────────┘
```

Note: Unlike the original framekernel design where frame and services are separate crates, Hadron uses a single `hadron-kernel` crate with the safety boundary enforced at the module level. The `arch/` modules contain the unsafe frame, and the rest of the crate contains safe services.

### Rules

1. **All `unsafe` code lives in `arch/`, `mm/`, and `sync/` modules**. Service modules (fs/, sched/, syscall/, proc/) avoid `unsafe` directly.
2. **Frame modules export safe APIs**. Even though their internals use `unsafe`, the public interface is safe Rust.
3. **Type-enforced invariants**. Newtypes like `PhysAddr`, `VirtAddr`, and `PhysFrame` encode hardware constraints in the type system.
4. **Trait-based abstraction**. Architecture-specific code implements common traits, so service modules are portable without being aware of the underlying platform.

### Why Not a Microkernel?

A microkernel achieves isolation through separate address spaces and IPC. A framekernel achieves isolation through Rust's type system — which has zero runtime cost. The tradeoffs:

| Property | Microkernel | Framekernel |
|----------|-------------|-------------|
| Isolation mechanism | Address spaces + IPC | Rust type system |
| Runtime cost | IPC overhead (context switches) | Zero (same address space) |
| Fault isolation | Process crash doesn't take down kernel | Unsafe bug in frame can corrupt everything |
| Language requirement | Any | Rust (or language with similar guarantees) |
| Auditability | Audit IPC interfaces | Audit `unsafe` blocks in frame |

The framekernel trades hardware-enforced isolation for compiler-enforced isolation, which is acceptable when:
- The frame modules are small and auditable
- The rest of the kernel benefits from zero-cost abstractions
- Performance is a priority (no IPC tax)

## Boot Flow

```
Bootloader (Limine/UEFI)
    │
    ▼
Boot Stub (kernel/boot/limine or kernel/boot/uefi)
    │  Constructs InitInfo from bootloader data
    ▼
hadron_kernel::kernel_init(boot_info)
    │  Initializes: GDT, IDT, PMM, VMM, services
    ▼
Idle loop / Init process
```

The boot stub is the bridge between the bootloader protocol and the kernel's internal `InitInfo` trait. This design allows multiple bootloaders (Limine, direct UEFI) without changing any kernel code.
