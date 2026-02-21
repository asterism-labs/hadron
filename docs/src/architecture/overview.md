# Architecture Overview

Hadron follows the **framekernel** architecture, a design that combines the performance characteristics of a monolithic kernel with the safety guarantees enabled by Rust's type system.

## What is a Framekernel?

A framekernel splits the kernel into two layers:

1. **The Frame** — A minimal unsafe core that directly interacts with hardware. This is the only code that uses `unsafe` Rust. It provides safe abstractions over hardware primitives.
2. **Safe Services** — The rest of the kernel, written entirely in safe Rust. Services implement high-level functionality (scheduler, filesystem, drivers) using the safe APIs exported by the frame.

This is conceptually similar to a microkernel's separation of concerns, but without the IPC overhead — both layers run in the same address space (ring 0) and communicate through direct function calls.

## Layer Architecture

| Layer | Crate | Safety | Responsibility |
|-------|-------|--------|----------------|
| **Frame** | `hadron-core` | Contains `unsafe` | Physical memory, page tables, interrupts, CPU state, I/O ports, context switching |
| **Safe Services** | `hadron-kernel` | Safe Rust only | Scheduler, VFS, syscall dispatch, drivers, networking |
| **Platform HAL** | `hadron-core/src/arch/` | `unsafe` in frame | Architecture-specific implementations behind common traits |
| **Boot Stubs** | `kernel/boot/{limine,uefi}/` | `unsafe` (entry) | Bootloader-specific entry, translates to `InitInfo` |
| **Reusable Crates** | `crates/{limine,noalloc,hadron-drivers,hadron-test,uefi}/` | Varies | Standalone no_std libraries and protocol bindings |

## Safety Model

The framekernel's key insight is using Rust's module visibility and type system as the safety boundary:

```
┌─────────────────────────────────────────────────────┐
│                    hadron-kernel                     │
│               (100% safe Rust services)             │
│                                                     │
│  ┌─────────┐ ┌──────┐ ┌─────────┐ ┌───────────┐   │
│  │Scheduler│ │ VFS  │ │Syscalls │ │  Drivers  │   │
│  └────┬────┘ └──┬───┘ └────┬────┘ └─────┬─────┘   │
│       │         │          │             │          │
├───────┴─────────┴──────────┴─────────────┴──────────┤
│              Safe API boundary (traits)              │
├─────────────────────────────────────────────────────┤
│                    hadron-core                       │
│              (unsafe frame, ~16% unsafe)            │
│                                                     │
│  ┌──────┐ ┌───────────┐ ┌─────┐ ┌──────────────┐  │
│  │ PMM  │ │Page Tables│ │ IDT │ │Context Switch│  │
│  └──────┘ └───────────┘ └─────┘ └──────────────┘  │
│                                                     │
│  ┌──────┐ ┌──────┐ ┌─────────┐ ┌─────────┐       │
│  │ GDT  │ │ APIC │ │ I/O Ops │ │ SYSCALL │       │
│  └──────┘ └──────┘ └─────────┘ └─────────┘       │
└─────────────────────────────────────────────────────┘
```

### Rules

1. **All `unsafe` code lives in `hadron-core`**. The `hadron-kernel` crate never uses `unsafe` directly.
2. **`hadron-core` exports only safe APIs**. Even though its internals use `unsafe`, the public interface is safe Rust.
3. **Type-enforced invariants**. Newtypes like `PhysAddr`, `VirtAddr`, and `PhysFrame` encode hardware constraints in the type system.
4. **Trait-based abstraction**. Architecture-specific code implements common traits, so `hadron-kernel` is portable without being aware of the underlying platform.

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
- The frame is small and auditable (~16% unsafe code)
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
