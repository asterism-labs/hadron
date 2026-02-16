# Completed Work (Phases 0-6)

Phases 0 through 6 of the Hadron kernel are complete. This document consolidates what was built in each phase, documents the key design decisions made along the way, and explains the most significant deviation from the original roadmap: choosing a cooperative async executor over a preemptive scheduler.

## Phase Summaries

### Phase 0: Build System & Boot Stub

Established the xtask build system with cross-compilation via `-Zbuild-std=core,compiler_builtins,alloc` targeting the custom `x86_64-unknown-hadron` target specification. Integrated the Limine boot protocol for early hardware setup and memory map handoff. The build pipeline produces a bootable ISO image and supports QEMU launch for rapid iteration. This phase defined the project's workspace structure and custom target JSON spec with soft-float, panic=abort, and disabled red zone.

### Phase 1: Serial Output & Early Console

Implemented a UART 16550 serial driver backed by port I/O wrappers (`inb`/`outb`). Introduced `SpinLock<T>` as the first synchronization primitive. Built `kprint!`/`kprintln!` macros for formatted kernel logging over the serial port. Added framebuffer text rendering using PSF bitmap fonts, providing a visible early console on the display output.

### Phase 2: CPU Initialization

Loaded a GDT with kernel code/data segments, user code/data segments, and a TSS entry. Built a 256-entry IDT with exception handlers for #DE (divide error), #DF (double fault), #GP (general protection), and #PF (page fault), each using IST stacks to guarantee a clean stack on fault. Installed a panic handler that halts the CPU after logging. The originally planned UEFI boot stub was deferred in favor of continuing with Limine.

### Phase 3: Physical Memory Management

Defined `PhysAddr` and `VirtAddr` newtypes with compile-time alignment and validity checks. Implemented HHDM (Higher Half Direct Map) translation between physical and virtual addresses. Built a bitmap frame allocator that tracks individual 4 KiB physical frames, initialized from the Limine-provided memory map.

### Phase 4: Virtual Memory & Kernel Heap

Implemented a 4-level page table walker and mapper operating through the HHDM. Defined `PageTableFlags` using the `bitflags` crate for precise control over page attributes (present, writable, no-execute, etc.). Set up a kernel heap region with a `#[global_allocator]` implementation, enabling use of `alloc` crate types (`Box`, `Vec`, `Arc`) throughout the kernel.

### Phase 5: Interrupts & Timers

Wrote a custom ACPI table parser (RSDP discovery, MADT parsing) to enumerate Local APICs and I/O APICs. Configured the Local APIC periodic timer with PIT-based calibration for tick-driven scheduling. Set up I/O APIC interrupt routing to redirect external IRQs to the correct CPU vectors. Implemented a PS/2 keyboard driver as the first external interrupt source.

### Phase 6: Async Executor

Implemented a priority-based cooperative async executor with three tiers: Critical, Normal, and Background. Built `WaitQueue` to bridge hardware interrupts into the async world, allowing tasks to `.await` IRQ events. Introduced `CpuLocal<T>` for per-CPU data storage in preparation for SMP. Added `LazyLock<T>` for safe lazy initialization of kernel statics. This phase deviated significantly from the original plan (see below).

## Key Design Decisions

**Framekernel architecture.** The kernel is split into an unsafe frame (`hadron-core`) that directly interacts with hardware and exports safe public APIs, and safe services (`hadron-kernel`) that contain all high-level logic. Both layers run in ring 0 with zero IPC overhead. This avoids the performance cost of a microkernel while preserving a strong safety boundary enforced by Rust's type system.

**Limine boot protocol.** Limine was chosen for its simplicity and broad QEMU support. It provides a consistent boot environment (memory map, framebuffer, HHDM offset) without requiring the kernel to implement UEFI or multiboot parsing. A direct UEFI boot stub remains possible but has been deferred.

**Bitmap PMM.** A simple one-bit-per-frame allocator was chosen for its minimal complexity. It tracks every 4 KiB frame with a single bit, making allocation O(n) in the worst case but trivial to implement and debug. This is sufficient for early phases; an upgrade to a buddy allocator can be done later without changing the allocator trait interface.

**HHDM (Higher Half Direct Map).** All physical memory is mapped at a fixed virtual offset provided by Limine. This allows direct pointer arithmetic to translate between physical and virtual addresses, eliminating the need for recursive page table mapping or temporary mappings when manipulating page tables.

**Async executor over preemptive scheduler.** The single largest deviation from the original roadmap. Rather than implementing a traditional preemptive round-robin scheduler, Phase 6 adopted Rust's native async/await model with a cooperative executor. The rationale is detailed in the next section.

## Deviation: Async Executor vs. Preemptive Scheduler

Phase 6 was originally planned as a preemptive round-robin scheduler with per-task kernel stacks and hand-written context switch assembly. Instead, a cooperative async executor was implemented. The decision was driven by four factors:

1. **Safe abstractions align with framekernel philosophy.** Rust's `Future`, `Pin`, and `Waker` are safe abstractions that compose without unsafe code. The executor itself lives in `hadron-kernel` (the safe services layer) with only the `CpuLocal<T>` and `WaitQueue` interrupt bridging requiring unsafe code in `hadron-core`. This directly supports the goal of minimizing the unsafe surface area.

2. **Simpler mental model.** A preemptive scheduler requires per-task kernel stacks, assembly context switch routines, stack guard pages, and careful management of register state across preemption points. The async executor eliminates all of this: each task is a state machine compiled by `rustc`, stored inline in a single allocation, with yield points at every `.await`.

3. **Natural SMP migration path.** Scaling to multiple CPUs requires only instantiating a `CpuLocal<Executor>` per core and adding a work-stealing queue between executors. This is substantially simpler than making a preemptive scheduler SMP-safe, which would require cross-CPU IPI-based preemption, per-CPU run queues with lock-free migration, and careful handling of interrupted critical sections.

4. **Async I/O is the modern approach.** The kernel's VFS, networking stack, and block device layers will all be implemented as async trait methods. Tasks waiting on I/O naturally yield to the executor rather than blocking a kernel thread, resulting in better CPU utilization without requiring a thread pool.

## Impact on Remaining Phases

The async executor model fundamentally changes the design of every phase after Phase 7. The following summarizes the key shifts:

**VFS (Phase 8).** `Inode` trait methods (`read`, `write`, `lookup`) return futures instead of blocking. A file read that requires disk I/O will `.await` the block device driver, yielding the executor to run other tasks while the DMA transfer completes.

**Userspace (Phase 9).** Each userspace process is backed by a kernel async task rather than a dedicated kernel thread. System calls enter the kernel, perform their work (potentially `.await`-ing I/O), and return to userspace. There are no per-process kernel stacks beyond the interrupt/syscall entry stack.

**IPC (Phase 11).** Traditional `fork()` is not implemented. Process creation uses `sys_spawn` with explicit argument passing. Inter-process communication uses async channels that integrate with the executor's waker mechanism, avoiding the need for blocking send/receive primitives.

**SMP (Phase 12).** Each CPU core runs its own `Executor` instance stored in `CpuLocal<T>`. Load balancing is achieved through work stealing: an idle executor can dequeue tasks from a busy executor's run queue. This phase has been moved earlier in priority given its natural fit with the per-CPU executor model.

**Networking (Phase 14).** The networking stack will integrate the `smoltcp` crate with async wrappers. Socket operations (`connect`, `send`, `recv`) are async methods that yield to the executor while waiting for packet arrival or transmission completion, avoiding dedicated network processing threads.
