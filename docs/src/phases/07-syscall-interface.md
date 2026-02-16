# Phase 7: Syscall Interface

**Status: COMPLETE**

## Goal

Implement the SYSCALL/SYSRET mechanism for user-to-kernel transitions, a syscall dispatch table, and UserPtr validation for safe access to user memory. After this phase, the kernel has a working syscall entry path and can dispatch basic system calls from both kernel-mode test callers and (eventually) ring 3 userspace.

## What Was Implemented

- MSR programming: STAR, LSTAR, SFMASK, EFER registers configured to enable SYSCALL/SYSRET.
- Assembly entry stub: `swapgs`, kernel stack switch, saves `rcx` (user RIP) and `r11` (user RFLAGS) plus callee-saved registers, calls `syscall_dispatch()`, returns via `sysretq`.
- `iretq` return path for kernel-mode callers (in `syscall.rs`).
- Guarded VMM-backed syscall stack allocation with guard pages (in `boot.rs`).
- Dispatch table with native Hadron syscall numbers (grouped by category).
- `UserPtr<T>` and `UserSlice` validation types (see below).
- `sys_clock_gettime` returning HPET-backed monotonic time via `Timespec`.
- `sys_debug_log`, `sys_task_exit`, `sys_task_info` handlers.
- `sys_mem_map` / `sys_mem_unmap` stubs (deferred to Phase 9).
- `ClockSource` trait implementation for HPET, registered as global driver instance.
- Kernel-mode integration tests for all implemented syscalls.

## Deferred Items

- `sys_mem_map` / `sys_mem_unmap` full implementation (Phase 9: user address spaces).

## Hybrid Syscall Model (Model C)

Not all syscalls need async treatment. The syscall interface uses a hybrid model:

- **Fast (synchronous) syscalls**: `getpid`, `brk`, `clock_gettime`. These complete immediately in the dispatch handler and return via `sysretq`. No executor involvement.
- **I/O (async-bridgeable) syscalls**: `read`, `write`, `open`, `close`. In Phase 9, when each process is backed by a kernel async task, these syscalls can `.await` VFS futures directly. The synchronous dispatch stub is sufficient until that point.

This avoids the overhead of futures for trivial operations while allowing I/O syscalls to participate in the async executor model introduced in Phase 6.

## UserPtr\<T\> Security

`UserPtr<T>` wraps a raw user-space pointer and validates it before dereferencing:

- Checks that the address is below `0x0000_8000_0000_0000` (canonical user half).
- Checks alignment for `T`.
- Checks that `addr + size_of::<T>()` does not overflow.
- Returns `Err(SyscallError::BadAddress)` on failure.

Kernel-mode test callers bypass validation via `is_kernel_caller()`, since their pointers reside in the upper half. This escape hatch is only used during early testing before userspace exists.

## Assembly Entry Stub

The entry stub is a `#[naked]` function installed as the LSTAR target:

1. `swapgs` -- switch GS base from user per-CPU area to kernel per-CPU area.
2. Save user RSP to per-CPU storage, load kernel syscall stack.
3. Push `rcx` (user RIP) and `r11` (user RFLAGS).
4. Push callee-saved registers (`rbp`, `rbx`, `r12`--`r15`).
5. Move `r10` to `rcx` (Linux convention: `r10` carries the 4th argument since `rcx` is clobbered by SYSCALL).
6. Call `syscall_dispatch(number, arg0..arg5)`.
7. Restore callee-saved registers, `r11`, `rcx`.
8. Restore user RSP.
9. `swapgs` and `sysretq`.

**Future**: An `iretq` return path is needed for signal delivery, where the kernel must modify the user return address before returning to ring 3.

## Syscall Numbers

The dispatch table uses native Hadron grouped syscall numbers:

| Number | Name | Model | Status |
|--------|------|-------|--------|
| 0x00 | task_exit | Terminal | Implemented |
| 0x05 | task_info | Fast | Implemented |
| 0x40 | mem_map | Synchronous | Stub (Phase 9) |
| 0x41 | mem_unmap | Synchronous | Stub (Phase 9) |
| 0x54 | clock_gettime | Fast | Implemented |
| 0xF1 | debug_log | Fast | Implemented |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| MSR programming (STAR, LSTAR, SFMASK, EFER) | Frame | Direct writes to CPU model-specific registers |
| `syscall_entry` assembly stub | Frame | Naked function, `swapgs`, stack switch |
| `UserPtr<T>` validation | Frame | Security boundary between user/kernel memory |
| Syscall number constants | Frame | Shared between frame and services |
| Dispatch table | Service | Match statement routing to safe handlers |
| `sys_read`, `sys_write`, etc. | Service | High-level syscall implementations |

## Dependencies

- **Phase 2**: GDT must define user code/data segments with RPL=3.
- **Phase 6**: Executor for per-CPU state (kernel syscall stack pointer stored in per-CPU area accessed via GS base).

## Milestone

Kernel-mode syscall test triggers the SYSCALL instruction and the handler dispatches correctly:

```rust
fn test_syscall() {
    let result: isize;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_GETPID,
            lateout("rax") result,
            out("rcx") _,
            out("r11") _,
        );
    }
    assert!(result > 0);
}
```

UserPtr validation rejects invalid pointers:

```rust
let bad_ptr: UserPtr<u8> = UserPtr::new(0xFFFF_8000_0000_0000 as *const u8);
assert!(bad_ptr.validate().is_err());

let good_ptr: UserPtr<u8> = UserPtr::new(0x0000_4000_0000_0000 as *const u8);
assert!(good_ptr.validate().is_ok());
```
