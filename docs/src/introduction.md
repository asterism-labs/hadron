# Introduction

Hadron is an x86_64 operating system kernel written in Rust. This document describes the second major revision of the kernel: a clean-room rewrite from a monolithic framekernel to a Zircon-style object microkernel.

## What Hadron Is

Hadron is a capability-based microkernel. Its central premise is that every kernel resource — processes, threads, address spaces, communication channels, memory regions, hardware interrupts — is represented as a typed kernel object. Userspace programs access these objects exclusively through handles: unforgeable capability tokens stored in a per-process table. A program that does not hold a handle to an object cannot interact with it, period.

The kernel itself provides only five foundational services:

1. **Object lifecycle management** — creation, reference counting, and destruction of kernel objects.
2. **Capability enforcement** — the handle table and rights system that mediates all object access.
3. **Inter-process communication** — channels, ports, sockets, FIFOs, and event primitives.
4. **Memory management** — physical page allocation, virtual address space management via VMOs and VMARs.
5. **Scheduling** — per-CPU executors that schedule threads, backed by the kernel's async runtime.

Everything else — device drivers, filesystems, networking, process supervision — runs in userspace as ordinary processes communicating with the kernel through syscalls and with each other through IPC channels.

## Why a Rewrite

The original Hadron kernel was a "framekernel": a monolithic design that kept all subsystems in a single kernel crate. The driver API was defined by traits compiled directly into the kernel binary, and the VFS was dispatched internally. This approach made early development fast but produced a design with several long-term problems:

- **Isolation failures**: A misbehaving driver could corrupt kernel memory. There was no hardware enforcement of driver boundaries.
- **Privilege creep**: Subsystems accumulated capabilities beyond what they strictly needed because there was no mechanism to express reduced-privilege access.
- **Inflexibility**: Adding a new filesystem or driver required recompiling and re-linking the kernel.
- **No formal capability model**: Object access was controlled by conventional Rust ownership rules at compile time, not by a runtime capability system that could be reasoned about from userspace.

The rewrite addresses all of these by moving every non-essential subsystem to userspace and enforcing all resource access through an explicit capability mechanism.

## Design Goals

**Security through capability isolation.** No process can access a resource it was not explicitly granted a handle to. Rights on handles are monotonically decreasing: a handle can be duplicated with fewer rights, but never more. The kernel enforces this invariant at every syscall boundary.

**Minimal trusted computing base.** The kernel binary must be as small as possible. The smaller the kernel, the smaller the surface area that must be trusted. Drivers, filesystems, and network stacks live in userspace; a bug in any of them cannot compromise the kernel.

**Explicit IPC over implicit sharing.** Processes share state by passing handles over channels, not by mapping shared memory implicitly. Shared memory (VMOs) is available when performance demands it, but the decision to share is explicit and capability-controlled.

**Simplicity of interfaces.** Syscalls are few and orthogonal. The object model is uniform: the same handle, rights, and signal mechanisms work identically across all object types.

**Driver isolation via IOMMU.** Hardware drivers running in userspace are granted DMA access only through Bus Transaction Initiator (BTI) objects backed by the IOMMU. A driver can only DMA to and from memory it has been explicitly granted, preventing DMA attacks.

## Relationship to Zircon

Hadron's object model, handle system, and IPC primitives are directly inspired by Zircon (the kernel of Fuchsia OS). The object taxonomy, rights semantics, channel message format, and port-based async signaling closely follow Zircon's design. However, Hadron is not a Zircon clone. The implementation is independent, the kernel is written in Rust rather than C++, and several design choices differ where the Rust ownership model or x86_64-specific concerns suggested a cleaner approach.

Readers familiar with Zircon will find the concepts map closely. Readers coming from Linux will find the biggest conceptual shift is the absence of file descriptors: handles fill this role, with capabilities enforced at the kernel rather than by POSIX DAC permission bits.
