# Threading & task_clone

**Status: Completed** (implemented in commit 38d1c33)

Hadron supports thread creation within a single process via the `task_clone` syscall, enabling multiple threads to share a single address space, file descriptors, and other process resources while maintaining independent execution contexts.

Source: [`kernel/hadron-kernel/src/proc/mod.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/hadron-kernel/src/proc/mod.rs), specifically the `Process::clone_thread()` method and [`kernel/hadron-kernel/src/syscall/process.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/hadron-kernel/src/syscall/process.rs)

## Thread Model

Threads in Hadron share the following process resources:

- **Address space** -- Same page table (CR3), enabling shared memory
- **File descriptor table** -- Open file descriptors visible to all threads
- **Signal handlers** -- Shared signal dispositions
- **TLS (Thread Local Storage)** -- Isolated via IA32_FS_BASE/IA32_GS_BASE MSR per thread

Each thread maintains independent:

- **Stack** -- Separate 64 KiB user stack in the shared address space
- **Registers** -- Independent CPU context (RIP, RSP, R8-R15, etc.)
- **TLS** -- Via `IA32_FS_BASE` and `IA32_GS_BASE` pointing to thread-local data

### Implementation Details

**Shared State via Arc:**

The kernel uses `Arc<SpinLock<T>>` for shared process state:

```rust
pub struct Process {
    pub address_space: Arc<SpinLock<AddressSpace<PageTableMapper>>>,
    pub fd_table: Arc<SpinLock<FileDescriptorTable>>,
    ...
}
```

When a thread is cloned, the `Arc` is cloned, incrementing the reference count. All threads share the same underlying address space and fd_table.

**Stack Allocation:**

Each thread gets its own 64 KiB stack allocated in the shared address space at a unique virtual address. Stack allocation uses a simple scheme: thread N gets a stack at `USER_STACK_TOP - (N * (64 KiB + 4 KiB guard))`.

## Syscall Interface

### sys_task_clone

```c
long sys_task_clone(unsigned long flags, void *stack_ptr, void *tls_ptr);
```

**Parameters:**

- **`flags`** -- Control which resources are shared:
  - **`CLONE_VM`** (0x100) -- Share address space (required for threads)
  - **`CLONE_FILES`** (0x400) -- Share file descriptor table (required for threads)
  - **`CLONE_SETTLS`** (0x80000) -- Set TLS pointer from `tls_ptr`
  
- **`stack_ptr`** -- User stack pointer for the new thread. Must point into the shared address space.

- **`tls_ptr`** -- TLS (thread-local storage) pointer. Written to `IA32_FS_BASE` MSR if `CLONE_SETTLS` is set.

**Return value:**

- **Parent**: Returns the new thread's task ID (positive integer).
- **Child**: Returns 0 (via RAX after entry).

### Thread Entry Point

The child thread begins execution at the user-provided stack pointer. The kernel initializes:

1. **RSP** -- Set to `stack_ptr` (user stack)
2. **RAX** -- Set to 0 (child returns 0)
3. **IA32_FS_BASE** or **IA32_GS_BASE** -- Set to `tls_ptr` if `CLONE_SETTLS` is set
4. **RIP** -- Entry point from the parent's return RIP (threads start at the same code location as the parent thread's next instruction)

## TLS (Thread-Local Storage)

Each thread can have its own TLS pointer, stored in either `IA32_FS_BASE` (X86_64_FS_BASED_TLS) or `IA32_GS_BASE` (X86_64_GS_BASED_TLS) MSR.

### TLS Structure

The TLS pointer typically points to a data structure like:

```c
struct tls {
    struct tls *self;    /* Points to itself */
    void *dtv;           /* Dynamic thread vector (for libc) */
    usize tid;           /* Thread ID */
    char *stack_base;    /* Stack base address */
    char *stack_end;     /* Stack end address */
};
```

Accessing `%fs:0` (in FS-based TLS) or `%gs:0` (in GS-based TLS) gives the thread-local data. The kernel does not interpret TLS; it merely sets the MSR. Userspace libraries like glibc manage TLS structure layout.

## Process vs Thread Distinction

**Processes** (`sys_task_spawn`):
- Create a new address space
- Do not share memory with the parent
- Have independent file descriptor tables (inherited/copied from parent)
- Are separate entries in the process table (different PID)

**Threads** (`sys_task_clone` with `CLONE_VM | CLONE_FILES`):
- Share the parent's address space
- Share the parent's file descriptor table
- Share the parent's process ID (all threads in a group are part of the same process)
- Have independent stacks and TLS

## Signal Delivery and Thread Groups

When a signal is delivered to a process with multiple threads:

- Signals are delivered to the **entire process group** (all threads).
- The kernel has flexibility in **which thread** handles the signal, though user code typically masks signals and explicitly calls `sigwait()`.

**Current limitation** (known issue): `execve()` in a multithreaded process is undefined. The syscall replaces the calling thread's address space but does not kill sibling threads, violating POSIX semantics.

## Implementation Status

Thread creation via `sys_task_clone`
Shared address space (`CLONE_VM`)
Shared file descriptors (`CLONE_FILES`)
Per-thread stacks
Per-thread TLS (`CLONE_SETTLS`, `IA32_FS_BASE` / `IA32_GS_BASE`)
Child returns 0, parent returns thread ID
Thread IDs and `tid` tracking
Thread list and `get_thread_list()` syscall
Multi-threaded `execve()` (POSIX compliance)
Thread-safe signal delivery

## Files to Modify

- `kernel/hadron-kernel/src/proc/mod.rs` -- Process and thread management
- `kernel/hadron-kernel/src/syscall/process.rs` -- `sys_task_clone` syscall handler
- `kernel/hadron-kernel/src/syscall/userptr.rs` -- User pointer validation for stack/TLS

## References

- **Process Management**: [Task Execution & Scheduling](../architecture/task-execution.md#process-model)
- **Memory & Address Space**: [Memory & Allocation](../architecture/memory.md)
- **Known Issues**: [Known Issues](../reference/known-issues.md#execve-in-multithreaded-processes-is-undefined)
