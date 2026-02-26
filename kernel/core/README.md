# hadron-core

Core types and synchronization primitives for the Hadron kernel, extracted into
a standalone `no_std` crate so they can be tested on the host with `cargo test`,
loom, and miri without requiring a kernel target. This crate provides the
foundational building blocks that the rest of the kernel and driver ecosystem
depends on: typed address wrappers, page/frame abstractions, task metadata,
per-CPU storage, type-safe resource identifiers, and a full suite of
synchronization primitives.

## Features

- **Typed addresses** -- `VirtAddr` and `PhysAddr` newtypes that enforce
  canonical-form invariants and prevent accidental mixing of virtual and
  physical addresses at the type level.
- **Page and frame abstractions** -- Generic `Page<S>` and `PhysFrame<S>` types
  parameterised over page size (4 KiB, 2 MiB, 1 GiB), with alignment
  guarantees and range iterators.
- **Synchronization primitives** -- `SpinLock`, `IrqSpinLock`, `Mutex` (async),
  `RwLock`, `SeqLock`, `Semaphore`, `Condvar`, `WaitQueue`,
  `HeapWaitQueue`, and `LazyLock`, all usable in `static` items and
  before any allocator or scheduler is available.
- **Lockdep** -- Optional lock dependency tracking enabled via the
  `hadron_lockdep` cfg, detecting potential deadlocks at runtime.
- **Task metadata** -- `TaskId`, `Priority` (Critical / Normal / Background),
  and `TaskMeta` with builder-pattern CPU affinity and priority configuration.
- **Scheduling primitives** -- Priority-aware `ReadyQueues` with per-tier FIFO
  scheduling, background starvation prevention, and work-stealing support
  with a one-task rule to prevent bouncing livelocks.
- **Per-CPU storage** -- `CpuLocal<T>` indexed by GS-based CPU ID on kernel
  targets, falling back to index 0 on host targets for single-threaded tests.
- **Type-safe identifiers** -- `Pid`, `CpuId`, `Fd`, and `IrqVector` newtypes
  that prevent accidental misuse of resource handles at compile time.
- **Utility macros** -- `static_assert!` for compile-time assertions and
  `RacyCell` for externally-synchronised statics.
