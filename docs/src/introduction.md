# Introduction

**Hadron** is a kernel written in Rust for the x86_64 architecture, following the **framekernel** design pioneered by [Asterinas](https://github.com/asterinas/asterinas). The project aims to build a practical, incrementally-developed operating system kernel that maximizes the use of safe Rust while confining `unsafe` code to a minimal, auditable core.

## Goals

- **Safety first**: Leverage Rust's type system and ownership model to eliminate entire classes of bugs (use-after-free, data races, buffer overflows) in the majority of kernel code.
- **Framekernel architecture**: Separate the kernel into an unsafe **frame** (`hadron-kernel::arch`) and safe **services** (the rest of `hadron-kernel`), providing a clear safety boundary without the IPC overhead of a microkernel.
- **Incremental POSIX compatibility**: Start with core syscalls (~50), expand to ~200 as features are added. Use Linux syscall numbers initially for easy testing with existing tools.
- **x86_64 first, architecture abstractions from day one**: Primary target is x86_64, but all arch-specific code lives behind traits so other architectures can be added later.
- **Minimal external dependencies**: Write our own UEFI, ACPI, and ELF crates as no_std, zero-dependency libraries. Only use external crates where the value is clear (e.g., `bitflags`).

## Key Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Architecture | Framekernel | Best safety/performance tradeoff for Rust kernels |
| Bootloader | Limine (primary), UEFI stub (secondary) | Limine provides HHDM and structured boot info; custom UEFI stub for learning |
| Syscall ABI | Unstable internal + stable userspace lib | Freedom to evolve kernel internals without breaking userspace |
| POSIX approach | Incremental | Start minimal, grow based on what applications actually need |
| Platform | x86_64 first | Most tooling/documentation available; arch traits ready for expansion |

## Current State

The project is in its early stages. What exists today:

- **Workspace structure**: `hadron-kernel` (monolithic kernel), `hadron-drivers` (pluggable drivers), `limine` crate, and build tool (`gluon`)
- **Limine protocol bindings**: Complete `crates/limine/` with request/response types, memory map, framebuffer, MP support
- **Kernel core**: `hadron-kernel` with `BootInfo` trait for boot handoff, arch abstractions, memory management, async executor, syscall interface
- **Linker script**: `targets/x86_64-unknown-hadron.ld` for ELF64 kernel image
- **Toolchain**: Nightly Rust targeting `x86_64-unknown-none` with `rust-src` and `llvm-tools-preview`

## About This Book

This book provides **architectural documentation** and a **development roadmap** for Hadron. It is organized into:

- **Architecture**: How the framekernel design works, crate layout, safety boundaries
- **Kernel Internals**: Subsystem documentation covering memory management, the async executor, VFS, syscalls, driver model, and more
- **Development Phases**: Remaining phases from async VFS to vDSO, each with goals, files, and milestones
- **Design Decisions**: Rationale behind syscall strategy, POSIX approach, memory layout, and architecture choice
- **Reference**: Target file tree, phase dependency graph, scope estimates
