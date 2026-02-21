# Kernel Architecture Comparison

This chapter compares the major kernel architecture approaches and explains why Hadron chose the framekernel model.

## Architecture Overview

### Monolithic Kernel

**Examples**: Linux, FreeBSD, OpenBSD

All kernel services (scheduler, VFS, drivers, networking) run in a single address space in ring 0. The entire kernel is one large program with full access to hardware.

```
┌────────────────────────────────────────┐
│            User Space (Ring 3)         │
├────────────────────────────────────────┤
│ ┌────────────────────────────────────┐ │
│ │           Monolithic Kernel         │ │
│ │  Scheduler │ VFS │ Drivers │ Net   │ │
│ │  Memory    │ IPC │ Security│ ...   │ │
│ │  (All in Ring 0, one address space) │ │
│ └────────────────────────────────────┘ │
├────────────────────────────────────────┤
│              Hardware                  │
└────────────────────────────────────────┘
```

**Pros**: Maximum performance (no IPC overhead), simple data sharing, well-understood model
**Cons**: Any bug can crash the entire kernel, large trusted computing base, harder to reason about safety

### Microkernel

**Examples**: seL4, MINIX 3, QNX, GNU Hurd

Only the absolute minimum runs in ring 0 (IPC, scheduling, basic memory management). Everything else (drivers, filesystem, networking) runs as separate userspace processes communicating via IPC.

```
┌────────────────────────────────────────────┐
│  User Space                                │
│  ┌──────┐ ┌─────┐ ┌───────┐ ┌──────────┐ │
│  │ VFS  │ │ Net │ │ Driver│ │  Driver  │ │
│  │Server│ │Stack│ │  (NIC)│ │  (Disk)  │ │
│  └──┬───┘ └──┬──┘ └───┬───┘ └────┬─────┘ │
│     │    IPC │   IPC  │   IPC    │        │
├─────┴────────┴────────┴──────────┴────────┤
│         Microkernel (Ring 0)               │
│    IPC │ Scheduler │ Basic VMM             │
├────────────────────────────────────────────┤
│              Hardware                      │
└────────────────────────────────────────────┘
```

**Pros**: Fault isolation (driver crash doesn't kill kernel), minimal trusted code, formal verification possible (seL4)
**Cons**: IPC overhead on every service interaction, complex programming model, driver performance penalty

### Framekernel

**Examples**: Asterinas

A Rust-specific architecture that splits the kernel into an **unsafe frame** and **safe services**, both running in ring 0. The Rust compiler enforces the safety boundary instead of hardware isolation.

```
┌────────────────────────────────────────┐
│           User Space (Ring 3)          │
├────────────────────────────────────────┤
│ ┌────────────────────────────────────┐ │
│ │          hadron-kernel              │ │
│ │  Scheduler │ VFS │ Drivers │ Net   │ │
│ │       (safe Rust services)         │ │
│ ├────────────────────────────────────┤ │
│ │    Unsafe Frame (arch/ modules)    │ │
│ │  PMM │ Paging │ IDT │ Ctx Switch  │ │
│ │       (contains unsafe)            │ │
│ └────────────────────────────────────┘ │
│        (All in Ring 0)                 │
├────────────────────────────────────────┤
│              Hardware                  │
└────────────────────────────────────────┘
```

**Pros**: Monolithic performance + safety guarantees from Rust type system, small auditable unsafe surface, zero IPC overhead
**Cons**: Requires Rust (or similar safe language), unsafe bug in frame can still corrupt everything, novel architecture with less ecosystem

### Unikernel

**Examples**: MirageOS, Unikraft, Nanos

A single-purpose kernel compiled together with one specific application into a single binary. No separation between kernel and user space.

**Pros**: Minimal attack surface, tiny footprint, fast boot, maximum performance
**Cons**: Single application only, no multi-tenancy, must recompile for each app

### Multikernel

**Examples**: Barrelfish

Treats each CPU core as an independent node. Cores communicate via message passing rather than shared memory. Designed for extreme heterogeneous hardware.

**Pros**: Scales to heterogeneous hardware, eliminates shared-memory bottlenecks
**Cons**: Very complex programming model, niche use case, small ecosystem

### Exokernel

**Examples**: MIT Exokernel (research)

Exposes raw hardware resources to applications with minimal abstraction. Applications (via "library OS") manage their own resources. The kernel only handles multiplexing and protection.

**Pros**: Maximum application control, zero abstraction overhead
**Cons**: Extremely complex application development, poor isolation between apps, purely academic

## Decision Matrix

| Property | Monolithic | Micro | Frame | Uni | Multi | Exo |
|----------|-----------|-------|-------|-----|-------|-----|
| **Performance** | +++++ | ++ | +++++ | +++++ | +++ | +++++ |
| **Safety** | + | ++++ | ++++ | ++ | +++ | + |
| **Fault isolation** | + | +++++ | ++ | N/A | ++++ | + |
| **Complexity** | +++ | ++ | +++ | ++++ | + | + |
| **Multi-app** | +++++ | +++++ | +++++ | — | +++++ | +++ |
| **Ecosystem** | +++++ | +++ | + | ++ | + | + |
| **Auditability** | + | ++++ | +++ | +++ | ++ | + |
| **Rust synergy** | ++ | +++ | +++++ | +++ | ++ | ++ |

*(+ = worse, +++++ = best)*

## Why Framekernel for Hadron

### 1. Rust's Type System is the Key Enabler

The framekernel architecture only works with a language that provides:
- Ownership and borrowing (prevents use-after-free)
- No null pointers (Option types instead)
- No data races (`Send`/`Sync` traits)
- Unsafe explicitly marked and auditable

Rust provides all of these. The compiler acts as the isolation mechanism — safer than a microkernel's IPC for most bug classes, and with zero runtime cost.

### 2. Monolithic Performance Without Monolithic Risk

A framekernel runs everything in ring 0 with direct function calls — identical to a monolithic kernel's performance profile. But unlike a monolithic kernel written in C, the vast majority of code (84%) physically cannot cause memory corruption because it's in safe Rust.

### 3. Practical Audit Surface

The audit target is clear and bounded: **only `unsafe` blocks in `hadron-kernel`'s frame modules (`arch/`, `mm/`, `sync/`)**. This is far smaller than auditing an entire monolithic kernel, and doesn't require the complex IPC infrastructure of a microkernel.

Estimated unsafe surface: ~16% of total kernel code, concentrated in:
- Page table manipulation
- Context switching
- Interrupt handling
- I/O port access
- APIC/MSR programming

### 4. Proven by Asterinas

Asterinas has demonstrated the framekernel approach works in practice, running real workloads with performance comparable to Linux for many operations. Hadron follows the same architectural principles.

### 5. Right Complexity Level for a Learning Project

A microkernel requires significant IPC infrastructure before any feature can be implemented. A monolithic kernel gives no structural guidance. The framekernel provides:
- Clear module boundaries (frame vs services)
- An obvious rule for where code goes (`unsafe` → frame, safe → service)
- Incremental complexity that scales with features

## Tradeoffs We Accept

| Tradeoff | Impact | Mitigation |
|----------|--------|------------|
| Unsafe bug in frame can corrupt everything | Same as monolithic for unsafe code | Keep frame small (~16%), audit carefully |
| No hardware fault isolation between services | A safe Rust bug (logic error) can affect other services | Rust prevents the most dangerous bug classes |
| Requires Rust expertise | Language learning curve | Rust is becoming mainstream; excellent documentation |
| Novel architecture, small ecosystem | Fewer resources and examples | Asterinas provides a reference implementation |
