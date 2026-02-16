# Crate Structure

Hadron is organized as a Cargo workspace with clear separation between kernel components, boot stubs, and reusable libraries.

## Workspace Layout

```
hadron/
├── Cargo.toml              # Workspace root (resolver = "2", edition = "2024")
├── rust-toolchain.toml     # Nightly + rust-src, llvm-tools-preview
│
├── kernel/
│   ├── hadron-core/        # The frame: unsafe core, safe public API
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── addr.rs         # PhysAddr, VirtAddr
│   │       ├── cell.rs         # Interior mutability primitives
│   │       ├── log.rs          # kprint!/kprintln! macros
│   │       ├── paging.rs       # High-level paging abstractions
│   │       ├── static_assert.rs
│   │       ├── arch/x86_64/    # instructions/, registers/, structures/
│   │       ├── mm/             # hhdm, pmm, vmm, heap, layout, region
│   │       └── sync/           # spinlock, rwlock, lazy
│   │
│   ├── hadron-kernel/      # Safe services: drivers, memory management
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── boot.rs         # Boot initialization
│   │       ├── log.rs          # Kernel logging
│   │       ├── sync.rs         # Synchronization wrappers
│   │       ├── arch/x86_64/    # gdt, idt, interrupts, paging
│   │       ├── drivers/        # early_fb (early framebuffer)
│   │       └── mm/             # heap, pmm, vmm, zone
│   │
│   └── boot/
│       └── limine/             # Limine bootloader entry point (binary crate)
│           ├── Cargo.toml
│           ├── build.rs
│           └── src/
│               ├── main.rs
│               └── requests.rs
│
├── crates/
│   ├── limine/             # Limine protocol bindings (no_std)
│   ├── noalloc/            # Allocation-free data structures (ringbuf, vec)
│   ├── hadron-drivers/     # Hardware drivers
│   ├── hadron-test/        # Test framework (QEMU exit, test runner)
│   ├── uefi/               # Custom UEFI bindings (Phase 2)
│   ├── acpi/               # ACPI table parsing (planned)
│   └── elf/                # ELF64 parser (planned)
│
└── xtask/                  # Build automation (runs on host)
    ├── Cargo.toml
    └── src/
        ├── main.rs         # CLI dispatch
        ├── build.rs        # Cross-compilation
        ├── config.rs       # Paths, targets
        ├── iso.rs          # ISO creation via xorriso + Limine
        ├── limine.rs       # Limine bootloader management
        ├── run.rs          # QEMU launch
        └── test.rs         # Test runner
```

## Crate Dependency Graph

```
kernel/boot/limine ──┬──> crates/limine
                     ├──> kernel/hadron-core
                     ├──> kernel/hadron-kernel
                     ├──> crates/hadron-drivers
                     └──> crates/noalloc

kernel/hadron-kernel ──┬──> kernel/hadron-core
                       ├──> crates/hadron-drivers
                       └──> crates/noalloc
                       (dev: hadron-test, limine)

kernel/hadron-core ──> bitflags

crates/limine    ──> (no deps)
crates/noalloc   ──> (no deps)
crates/uefi      ──> (no deps)

xtask ──> (host-only, uses std)
```

Key design principle: **`crates/*` are standalone, no_std libraries** that can be tested independently and reused in other projects. `hadron-core` depends on `bitflags` for hardware register flag definitions.

## Crate Responsibilities

### `kernel/hadron-core` — The Frame

The unsafe foundation. Contains all hardware interaction code and exports safe abstractions.

| Module | Purpose |
|--------|---------|
| `addr.rs` | `PhysAddr`, `VirtAddr` newtypes |
| `cell.rs` | Interior mutability primitives for kernel use |
| `log.rs` | `kprint!`/`kprintln!` macros |
| `paging.rs` | High-level paging abstractions |
| `static_assert.rs` | Compile-time assertions |
| `arch/x86_64/instructions/` | Port I/O, interrupts, segmentation, TLB, tables |
| `arch/x86_64/registers/` | Control registers, MSRs, RFLAGS |
| `arch/x86_64/structures/` | GDT, IDT, paging structures |
| `mm/` | HHDM, PMM, VMM, heap, memory layout, regions |
| `sync/` | `SpinLock`, `RwLock`, `Lazy` |

### `kernel/hadron-kernel` — Safe Services

High-level kernel functionality built on the frame's safe abstractions.

| Module | Purpose |
|--------|---------|
| `boot.rs` | Boot initialization sequence |
| `log.rs` | Kernel logging infrastructure |
| `sync.rs` | Synchronization wrappers |
| `arch/x86_64/` | GDT, IDT, interrupts, paging setup |
| `drivers/` | Early framebuffer driver |
| `mm/` | Heap allocator, PMM, VMM, zone allocator |

### `crates/*` — Reusable Libraries

| Crate | Purpose | Status |
|-------|---------|--------|
| `limine` | Limine boot protocol bindings | Exists |
| `noalloc` | Allocation-free data structures (ring buffer, array vec) | Exists |
| `hadron-drivers` | Hardware drivers | Exists |
| `hadron-test` | Test framework (QEMU isa-debug-exit, test runner) | Exists |
| `uefi` | UEFI protocol bindings (SystemTable, GOP, etc.) | Phase 2 |
| `acpi` | ACPI table parsing (RSDP, MADT, HPET, FADT) | Planned |
| `elf` | ELF64 parsing (program headers, entry point) | Planned |

### `kernel/boot/*` — Boot Stubs

Binary crates that bridge bootloader protocols to the kernel.

| Stub | Crate Name | Bootloader | Purpose |
|------|-----------|-----------|---------|
| `kernel/boot/limine/` | `hadron-boot-limine` | Limine | Primary boot path, HHDM provided by bootloader |
| `kernel/boot/uefi/` | — | Direct UEFI | Alternative boot (Phase 2, planned) |

### `xtask` — Build Automation

Host-side tool providing:

| Command | Purpose |
|---------|---------|
| `cargo xtask build` | Cross-compile kernel for target architecture |
| `cargo xtask run` | Build + create ISO + launch QEMU |
| `cargo xtask test` | Build + run tests in QEMU with isa-debug-exit |
