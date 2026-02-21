# Process Management

Hadron models each user process as an async task on the kernel's cooperative
executor. A process owns its address space, file descriptor table, and
identity (PID, parent PID). The kernel enters and exits user mode through a
setjmp/longjmp mechanism built on `iretq` and saved kernel stack pointers,
with timer-driven preemption providing fair scheduling between processes.

Source locations referenced in this chapter live under
`kernel/hadron-kernel/src/`.

## Process struct

The core type is `proc::Process`, defined in `proc/mod.rs`:

```rust
pub struct Process {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub user_cr3: PhysAddr,
    address_space: AddressSpace<PageTableMapper>,
    pub fd_table: SpinLock<FileDescriptorTable>,
    pub exit_status: SpinLock<Option<u64>>,
    pub exit_notify: HeapWaitQueue,
}
```

Key fields:

- **`pid`** -- monotonically assigned from an `AtomicU32` (`NEXT_PID`).
- **`parent_pid`** -- `None` for the init process, `Some(pid)` for children.
- **`user_cr3`** -- cached physical address of the process PML4, used for
  fast CR3 switches without re-reading the address space struct.
- **`address_space`** -- an `AddressSpace<PageTableMapper>` that owns the
  per-process PML4 frame. Freed automatically via `Drop`.
- **`fd_table`** -- a `SpinLock<FileDescriptorTable>` holding the mapping
  from integer fd numbers to `FileDescriptor` entries (inode + offset + flags).
- **`exit_status` / `exit_notify`** -- used for the `sys_task_wait` mechanism.
  When a process exits, its status is stored and all waiters are notified via
  the `HeapWaitQueue`.

### Global process table

All live processes are tracked in `PROCESS_TABLE`, a `SpinLock<BTreeMap<u32,
Arc<Process>>>`. Three functions operate on it:

- `register_process()` -- inserts a process at spawn time.
- `lookup_process(pid)` -- returns `Arc<Process>` by PID.
- `unregister_process(pid)` -- removes a reaped process (called by
  `handle_wait` after the parent collects the exit status).

Exited processes remain in the table as zombies until the parent calls
`sys_task_wait`, which reaps them.

## ELF loading (binfmt)

Binary loading uses a trait-based format registry in `proc/binfmt/mod.rs`.

### The `BinaryFormat` trait

```rust
pub trait BinaryFormat: Sync {
    fn name(&self) -> &'static str;
    fn probe(&self, data: &[u8]) -> bool;
    fn load<'a>(&self, data: &'a [u8]) -> Result<ExecImage<'a>, BinaryError>;
}
```

Registered handlers are tried in order. The static registry currently contains
two entries:

1. **`ElfHandler`** (`proc/binfmt/elf.rs`) -- handles ELF64 binaries.
2. **`ScriptHandler`** (`proc/binfmt/script.rs`) -- recognises `#!` shebangs
   but is not yet implemented (returns `BinaryError::Unimplemented`).

The top-level `load_binary(data)` function probes each handler and delegates
to the first match.

### `ExecImage` and `ExecSegment`

A successful load returns an `ExecImage`:

```rust
pub struct ExecImage<'a> {
    pub entry_point: u64,
    pub base_addr: u64,
    pub needs_relocation: bool,
    pub elf_data: Option<&'a [u8]>,
    segments: ArrayVec<ExecSegment<'a>, 16>,
}
```

Each `ExecSegment` carries zero-copy borrows of the file data, a virtual
address, total memory size, and permission flags (`writable`, `executable`).
Up to 16 loadable segments are supported.

### ELF handler details

The ELF handler (`proc/binfmt/elf.rs`) supports two ELF types:

- **`ET_EXEC`** (fixed-address) -- segments map at their stated vaddrs,
  `base_addr = 0`, no relocation needed.
- **`ET_DYN`** (static-PIE) -- segments are offset by `USER_PIE_BASE`
  (`0x40_0000`), and the image is flagged for `.rela.dyn` relocation.

`ET_REL` (relocatable objects) is rejected; those are intended for a separate
kernel module loader path.

### Relocation

For `ET_DYN` binaries, `proc/binfmt/reloc.rs` applies `.rela.dyn` entries
after segments are mapped. The function `apply_dyn_relocations()`:

1. Locates the symbol table (`.dynsym` preferred, `.symtab` fallback).
2. Iterates all `SHT_RELA` sections.
3. Resolves each symbol (index 0 returns 0 for `R_X86_64_RELATIVE`; defined
   symbols return `st_value + base_addr`; undefined symbols error).
4. Computes the relocation value via `hadron_elf::compute_x86_64_reloc`.
5. Writes the result (`u32` or `u64`) into the already-mapped user page
   through the HHDM.

## Process creation

The function `proc::exec::create_process_from_binary()` orchestrates the full
sequence:

1. **Parse** -- calls `binfmt::load_binary(data)` to get an `ExecImage`.
2. **Address space** -- allocates a new `AddressSpace` via
   `AddressSpace::new_user()`, which allocates a PML4 frame, zeroes the lower
   half (entries 0-255), and copies the kernel upper half (entries 256-511)
   from the kernel PML4.
3. **Map segments** -- iterates `ExecImage::segments()` and maps each one
   page-by-page. Each page is allocated from the PMM, zeroed, and populated
   with file data via HHDM pointer arithmetic. Permission flags (`USER`,
   `WRITABLE`, `EXECUTABLE`) are applied per-segment.
4. **Relocate** -- if the image is `ET_DYN`, applies `.rela.dyn` entries.
5. **Map stack** -- allocates a 64 KiB (16-page) user stack at
   `USER_STACK_TOP` (`0x7FFF_FFFF_F000`), growing downward. All stack pages
   are writable + user-accessible.
6. **Return** -- wraps the address space in a `Process` and returns the
   entry point and stack top.

### Argv setup

After process creation, `write_argv_to_stack()` writes argument strings and
a Rust-native `(ptr, len)` array onto the user stack via HHDM translation.
The stack layout at entry is:

```text
HIGH ADDRESS (0x7FFF_FFFF_F000)
  +-----------------------------+
  | arg string bytes (UTF-8)    |
  +-----------------------------+
  | (ptr, len) pairs, 16 bytes  |
  | each, directly castable to  |
  | &[&str]                     |
  +-----------------------------+
  | argc: usize                 |  <- RSP (16-byte aligned)
  +-----------------------------+
```

## Spawning

Two spawn paths exist:

- **`spawn_init()`** -- reads `/init` from the VFS, creates a process with no
  parent, sets up fd 0/1/2 on `/dev/console`, writes `["/init"]` as argv, and
  spawns the async `process_task`.
- **`spawn_process(path, parent_pid, args)`** -- general spawn called by
  `sys_task_spawn`. Reads the binary from the VFS, creates a process with
  `parent_pid`, inherits fd 0/1/2 from the parent (or falls back to
  `/dev/console`), writes argv, registers in the process table, and spawns
  the async `process_task`.

Both paths end with `sched::spawn(process_task(...))`, placing the process
task on the async executor's run queue.

## Userspace entry and exit

The kernel enters and exits ring 3 through a **setjmp/longjmp pattern**
built on naked assembly functions in `arch/x86_64/userspace.rs`.

### Initial entry: `enter_userspace_save`

For a process's first entry, `enter_userspace_first()` in `proc/mod.rs`:

1. Disables interrupts (`cli`).
2. Saves the kernel per-CPU GS base into `IA32_KERNEL_GS_BASE`, zeroes
   `IA32_GS_BASE` (so user code gets a clean GS).
3. Loads the process's PML4 into CR3.
4. Calls `enter_userspace_save(entry, stack_top, saved_rsp_ptr)`.

`enter_userspace_save` is a naked function that acts as the **setjmp** half:

1. Pushes callee-saved registers (RBP, RBX, R12-R15) onto the kernel stack.
2. Writes the current RSP to `*saved_rsp_ptr` (stored in
   `SAVED_KERNEL_RSP`).
3. Builds an `iretq` frame (SS, RSP, RFLAGS, CS, RIP with user-mode
   selectors) and zeroes all GPRs to prevent information leaks.
4. Executes `iretq` to transition to ring 3.

### Return to kernel: `restore_kernel_context`

When a syscall, fault, or timer preemption needs to return control to the
kernel process task, it calls `restore_kernel_context(saved_rsp)` -- the
**longjmp** half:

1. Sets RSP to the saved kernel stack pointer.
2. Pops the callee-saved registers pushed by `enter_userspace_save`.
3. Executes `ret`, which returns into the process task as if
   `enter_userspace_save` had returned normally.

Before calling `restore_kernel_context`, the handler must:

- Restore kernel CR3 (`Cr3::write(kernel_cr3())`).
- Restore GS bases so both `GS_BASE` and `KERNEL_GS_BASE` point to the
  per-CPU data.
- Set `TRAP_REASON` to indicate why userspace was interrupted.

### Resume after preemption: `enter_userspace_resume`

When a preempted process is rescheduled, `enter_userspace_resume_wrapper()`
calls `enter_userspace_resume(ctx, saved_rsp_ptr)`, which:

1. Pushes callee-saved registers and saves RSP (same as the initial path).
2. Builds an `iretq` frame from the saved `UserRegisters` context
   (RIP, RSP, RFLAGS from the `USER_CONTEXT` struct).
3. Restores all 15 GPRs from the context.
4. Executes `iretq`.

## Context saving

User-mode state is saved differently depending on the trap reason.

### Syscalls (`SYSCALL` instruction)

The `syscall_entry` naked function in `arch/x86_64/syscall.rs` handles the
x86-64 `SYSCALL` mechanism. On entry, the CPU saves RIP into RCX and RFLAGS
into R11, and loads kernel CS/SS from STAR. The stub then:

1. `swapgs` to get the per-CPU GS base.
2. Saves user RSP to `percpu.user_rsp` (GS:[16]).
3. Loads the kernel RSP from `percpu.kernel_rsp` (GS:[8]).
4. Pushes return RIP (RCX), RFLAGS (R11), and callee-saved registers.
5. Copies callee-saved regs + RIP + RFLAGS to `SYSCALL_SAVED_REGS`
   (a per-CPU `SyncSavedRegs` struct accessed via GS:[56]).
6. Remaps the Linux syscall register convention to SysV C calling convention
   and calls `syscall_dispatch`.
7. On return, pops registers and uses `sysretq` (user) or `iretq` (kernel).

The `SYSCALL_SAVED_REGS` snapshot is needed because blocking syscalls
(`sys_task_wait`, pipe I/O) longjmp via `restore_kernel_context`, which
destroys the syscall stack. When the process resumes, `process_task`
reconstructs `USER_CONTEXT` from the snapshot.

### Timer preemption (LAPIC vector 254)

The `timer_preempt_stub` in `arch/x86_64/interrupts/timer_stub.rs` handles
two cases:

- **Ring 0 interrupts** -- standard save/dispatch/restore/`iretq` (no
  preemption).
- **Ring 3 interrupts** -- full preemption:
  1. `swapgs` to get kernel GS.
  2. Saves all 15 GPRs + interrupt frame RIP/RSP/RFLAGS into `USER_CONTEXT`
     via the per-CPU pointer at GS:[32].
  3. Calls `timer_tick_and_eoi`.
  4. Restores kernel CR3 from `KERNEL_CR3`.
  5. Fixes GS bases.
  6. Sets `TRAP_REASON = TRAP_PREEMPTED`.
  7. Inlines `restore_kernel_context` to longjmp back to `process_task`.

### Faults

`terminate_current_process_from_fault()` in `proc/mod.rs` restores kernel
CR3 and GS bases, sets `TRAP_REASON = TRAP_FAULT` with exit status
`usize::MAX`, and calls `restore_kernel_context`.

## The process task event loop

Each process is driven by `process_task()`, an async function in
`proc/mod.rs`. It runs a loop:

1. Sets `CURRENT_PROCESS` to the running process.
2. Enters userspace (first entry or resume).
3. On return, clears `CURRENT_PROCESS` and reads `TRAP_REASON`.
4. Dispatches on the trap reason:

| Trap reason      | Constant        | Action |
|------------------|-----------------|--------|
| `TRAP_EXIT`      | 0               | Log exit status, store it, notify waiters, break. |
| `TRAP_PREEMPTED` | 1               | Snapshot `USER_CONTEXT`, `yield_now().await`, restore context, continue. |
| `TRAP_FAULT`     | 2               | Log fault, store `usize::MAX` exit status, notify waiters, break. |
| `TRAP_WAIT`      | 3               | Snapshot saved regs, `handle_wait().await`, write exit status to user memory, rebuild `USER_CONTEXT`, continue. |
| `TRAP_IO`        | 4               | Snapshot saved regs, perform async read/write on the target inode, copy data across CR3 boundary, rebuild `USER_CONTEXT`, continue. |

For blocking traps (`TRAP_WAIT`, `TRAP_IO`), the task snapshots the
`SYSCALL_SAVED_REGS` and `percpu.user_rsp` before yielding, because these
per-CPU statics will be overwritten by other processes while this task is
suspended. After the `.await` completes, the task reconstructs `USER_CONTEXT`
from the snapshot and continues into `enter_userspace_resume_wrapper`.

## Per-process address space

Each process has its own PML4 page table, managed by `AddressSpace` in
`mm/address_space.rs`.

### Creation

`AddressSpace::new_user()` allocates a fresh PML4 frame and initializes it:

- **Lower half** (entries 0-255) -- zeroed, reserved for user mappings.
- **Upper half** (entries 256-511) -- copied from the kernel PML4, sharing
  the kernel's page table subtrees.

This means kernel code and data are accessible in every process address space
(at ring 0 only), which is necessary for syscall handlers that run in the
process's CR3 before switching back to the kernel CR3.

### CR3 switching

On entry to userspace, CR3 is loaded with `process.user_cr3`. On exit
(syscall, fault, preemption), the handler restores the kernel CR3 from the
global `KERNEL_CR3` atomic, which was saved once during early boot via
`save_kernel_cr3()`.

### Cleanup

`AddressSpace` implements `Drop`. When the last `Arc<Process>` reference is
released, the `Drop` impl calls the stored `dealloc_fn` callback, which
returns the PML4 frame to the bitmap frame allocator. The `Process::Drop`
impl logs the deallocation.

## Per-CPU state

Process management relies on several per-CPU statics, all accessed through
the `CpuLocal<T>` wrapper indexed by CPU ID:

| Static               | Type                          | Purpose |
|----------------------|-------------------------------|---------|
| `SAVED_KERNEL_RSP`   | `CpuLocal<AtomicU64>`         | Kernel RSP saved by `enter_userspace_save` for `restore_kernel_context`. |
| `USER_CONTEXT`       | `CpuLocal<SyncUserContext>`   | Full `UserRegisters` saved by the timer preemption stub. |
| `TRAP_REASON`        | `CpuLocal<AtomicU8>`          | Why userspace was interrupted (exit / preempt / fault / wait / io). |
| `CURRENT_PROCESS`    | `CpuLocal<SpinLock<Option<Arc<Process>>>>` | Currently running process, accessed by syscall handlers. |
| `PROCESS_EXIT_STATUS`| `CpuLocal<AtomicU64>`         | Exit status written before `restore_kernel_context`. |
| `WAIT_TARGET_PID`    | `CpuLocal<AtomicU32>`         | Target PID for `sys_task_wait`. |
| `IO_FD` / `IO_BUF_*` | `CpuLocal<AtomicU64>`        | Parameters for blocking I/O traps. |

The `PerCpu` struct (in `percpu.rs`) also stores pointers to `USER_CONTEXT`,
`SAVED_KERNEL_RSP`, `TRAP_REASON`, and `SYSCALL_SAVED_REGS` at fixed offsets
so that assembly stubs can access them directly via `GS:[offset]` without
calling into Rust.
