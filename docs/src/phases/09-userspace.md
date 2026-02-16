# Phase 9: Userspace & ELF Loading

## Goal

Load and execute ELF64 binaries in ring 3. Each user process is backed by a kernel async task on the executor -- there are no per-process kernel threads and no preemptive thread scheduler. After this phase, the kernel runs its first userspace program: a minimal static ELF that prints a message via the write syscall and exits.

## Process Model: Async Tasks

Each process is represented as an async task spawned on the executor. The task loop enters userspace and handles the result -- which may be a syscall, a preemption event, or a fault:

```rust
/// Return type from enter_userspace(). The kernel async state machine
/// is never "interrupted" -- enter_userspace() is a regular function
/// that returns one of three variants.
enum UserspaceReturn {
    Syscall(SyscallArgs),
    Preempted,
    Fault(FaultInfo),
}

async fn process_task(process: Arc<Process>) {
    loop {
        match enter_userspace(&process) {
            UserspaceReturn::Syscall(args) => {
                match handle_syscall(&process, args).await {
                    SyscallResult::Continue(ret) => set_return_value(ret),
                    SyscallResult::Exit(status) => {
                        process.exit_notify.wake_all();
                        return;
                    }
                }
            }
            UserspaceReturn::Preempted => {
                // User state already saved on interrupt frame.
                // Yield to let other tasks run; when re-polled,
                // enter_userspace() restores saved state via iretq.
                yield_now().await;
            }
            UserspaceReturn::Fault(info) => {
                handle_fault(&process, info).await;
            }
        }
    }
}
```

Key properties of this model:

- I/O syscalls `.await` VFS futures directly inside `handle_syscall()`. For ramfs, these resolve immediately. For block-backed filesystems, the task yields to the executor until I/O completes.
- Fast syscalls (getpid, brk, clock_gettime) do not await and return synchronously within the same poll cycle.
- No `fork()`. Process creation happens exclusively through `exec()`, which spawns a new async task.

### Timer-Driven Preemption

When a timer interrupt fires while user code runs in ring 3, the CPU traps to the kernel. The timer handler sets a `PerCpu::preempt_current` flag. The interrupt return path checks this flag; if set, it returns to the executor loop instead of back to userspace. The `process_task` then sees `UserspaceReturn::Preempted` and calls `yield_now().await` to re-queue itself.

This gives the kernel timer-driven preemption of userspace code without any ability to "interrupt" a Rust Future mid-poll. See [Preemption & Scaling](../design/preemption-and-scaling.md#userspace-preemption-timer-driven-phase-9) for the full design.

## Process Creation via exec()

```rust
impl Process {
    pub fn exec(
        elf_data: &[u8],
        args: &[&str],
        envp: &[&str],
    ) -> Result<Arc<Process>, ExecError> {
        let elf = Elf64::parse(elf_data)?;
        let mut address_space = AddressSpace::new_user(&kernel_address_space());

        // Map LOAD segments into user address space
        for segment in elf.load_segments() {
            let flags = segment_flags_to_page_flags(segment.flags);
            // Allocate frames, map pages, copy segment data
            // Zero-fill BSS region (memsz - filesz)
        }

        // Set up user stack
        let stack_top = setup_user_stack(&mut address_space, args, envp, &elf)?;

        let process = Arc::new(Process {
            address_space,
            fd_table: Mutex::new(FileDescriptorTable::new_with_stdio()),
            pid: next_pid(),
            exit_notify: WaitQueue::new(),
        });

        // Spawn the process as an async task on the executor
        executor::spawn(process_task(process.clone()), Priority::Normal);

        Ok(process)
    }
}
```

## ELF Parser

The standalone `crates/hadron-elf/` crate (already exists) provides ELF64 parsing:

- Validates magic bytes, 64-bit class, x86_64 machine type.
- Parses ELF header and program header table.
- Iterates LOAD segments with virtual address, file data, memory size, and permission flags.
- No unsafe code -- pure byte-level parsing.

## User Address Space

A new PML4 is created for each process. The upper half (kernel space) is shared across all address spaces by copying the kernel's PML4 entries for addresses above `0xFFFF_8000_0000_0000`. The lower half is private to the process:

- LOAD segments mapped at their specified virtual addresses with appropriate R/W/X permissions.
- User stack mapped at `0x7FFF_FFFF_F000` (top), growing downward. Default size: 8 MiB with a guard page.

### User Stack Layout

```
High addresses (USER_STACK_TOP = 0x7FFF_FFFF_F000)
+---------------------------+
| null terminator padding   |
+---------------------------+
| environment strings       |  "PATH=/bin\0"
| argument strings          |  "./init\0"
+---------------------------+
| auxv[N] = {AT_NULL, 0}    |  Auxiliary vector terminator
| auxv[1] = {AT_ENTRY, ...} |
| auxv[0] = {AT_PHDR, ...}  |
+---------------------------+
| NULL                      |  envp terminator
| envp[0]                   |  Pointer to env string
+---------------------------+
| NULL                      |  argv terminator
| argv[0]                   |  Pointer to arg string
+---------------------------+
| argc                      |  Argument count
+---------------------------+ <-- RSP (stack pointer on entry)
```

## Jump to Userspace

The ring 0 to ring 3 transition is performed by constructing an `iretq` frame on the kernel stack:

```rust
/// Transition to ring 3 by constructing an iret frame.
///
/// Pushes SS, RSP, RFLAGS (IF=1), CS, RIP onto the kernel stack,
/// clears all general-purpose registers for security, then executes iretq.
#[naked]
pub unsafe extern "C" fn jump_to_userspace(
    entry: u64,
    user_rsp: u64,
) -> ! {
    core::arch::naked_asm!(
        "push 0x1B",       // SS = user_data (0x18 | RPL=3)
        "push rsi",        // RSP (user stack pointer)
        "push 0x202",      // RFLAGS (IF=1)
        "push 0x23",       // CS = user_code (0x20 | RPL=3)
        "push rdi",        // RIP (entry point)
        // Clear all GPRs to prevent kernel data leakage
        "xor rax, rax",
        "xor rbx, rbx",
        "xor rcx, rcx",
        "xor rdx, rdx",
        "xor rsi, rsi",
        "xor rdi, rdi",
        "xor rbp, rbp",
        "xor r8, r8",
        "xor r9, r9",
        "xor r10, r10",
        "xor r11, r11",
        "xor r12, r12",
        "xor r13, r13",
        "xor r14, r14",
        "xor r15, r15",
        "iretq",
    );
}
```

## Process Struct

```rust
pub struct Process {
    pub address_space: AddressSpace,
    pub fd_table: Mutex<FileDescriptorTable>,
    pub pid: u32,
    pub exit_notify: WaitQueue,
}
```

The `exit_notify` WaitQueue is signaled when the process exits, allowing `sys_waitpid` (Phase 11) to await process termination.

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| ELF crate | Crate (standalone) | Pure byte parsing, no unsafe |
| `jump_to_userspace()` | Frame | Naked assembly, constructs iret frame |
| `Process` struct | Service | High-level process management |
| `exec()` implementation | Service | Uses safe address space and ELF APIs |
| `process_task()` async loop | Service | Executor integration, syscall handling |
| User stack setup | Service | Memory writes through safe mapper API |

## Dependencies

- **Phase 7**: Syscall interface (for write and exit syscalls).
- **Phase 8**: VFS (for loading `/init` from initramfs, stdio file descriptors).
- **Phase 4**: Virtual memory (for user address space creation).

## Milestone

The first userspace program runs:

```
Loading /init from initramfs...
Process 1 (init): entering userspace at 0x400000
Hello from userspace!
Process 1 (init): exited with status 0
```

The init binary is a minimal statically-linked ELF:

```asm
; Compiled as a static ELF with no libc
_start:
    mov rax, 0xF1     ; SYS_DEBUG_LOG
    lea rdi, [msg]    ; buffer
    mov rsi, 22       ; length
    syscall
    mov rax, 0x00     ; SYS_TASK_EXIT
    xor rdi, rdi      ; status = 0
    syscall
msg: .ascii "Hello from userspace!\n"
```
