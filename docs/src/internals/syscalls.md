# Syscall Interface

Hadron uses the x86_64 `SYSCALL`/`SYSRET` fast-path for all userspace-to-kernel
transitions. Syscall definitions are centralized in the `hadron-syscall` crate
using a custom DSL macro (`define_syscalls!`), which generates constants,
dispatch logic, and userspace stubs from a single source of truth.

## Architecture overview

The syscall subsystem spans three crates:

| Crate | Role |
|---|---|
| `hadron-syscall-macros` | Proc macro crate: parses the DSL and generates code |
| `hadron-syscall` | Single source of truth: syscall numbers, error codes, `#[repr(C)]` types, and feature-gated kernel/userspace code |
| `hadron-kernel` (`syscall/`) | Kernel-side handler implementations organized by category |

The `hadron-syscall` crate uses Cargo features to control what gets compiled:

- **`kernel`** feature: emits the `SyscallHandler` trait and `dispatch()` function.
- **`userspace`** feature: emits raw `syscallN` inline assembly stubs (`raw` module) and typed wrapper functions (`wrappers` module).
- **Always emitted**: `SYS_*` number constants, `E*` error constants, `#[repr(C)]` shared types, `Syscall` and `SyscallGroup` enums.

## SYSCALL/SYSRET entry mechanism

Initialization and the entry stub live in `arch/x86_64/syscall.rs`. The `init()`
function programs four MSRs after GDT and per-CPU setup:

| MSR | Value | Purpose |
|---|---|---|
| `IA32_EFER` | Set `SCE` bit | Enable `SYSCALL`/`SYSRET` instructions |
| `STAR` | `0x08` (bits 32-47), `0x10` (bits 48-63) | Kernel CS/SS for `SYSCALL`; base for `SYSRET` CS/SS |
| `LSTAR` | Address of `syscall_entry` | Entry point the CPU jumps to on `SYSCALL` |
| `SFMASK` | `0x600` (IF + DF) | RFLAGS bits masked on entry -- disables interrupts and clears direction flag |

When userspace executes the `SYSCALL` instruction, the CPU saves RIP into RCX
and RFLAGS into R11, loads kernel CS/SS from STAR, and jumps to LSTAR. It does
**not** switch RSP -- the kernel must do that manually.

### Entry stub (`syscall_entry`)

The naked assembly function `syscall_entry` performs the following steps:

1. **Switch to kernel context**: `swapgs` to load kernel GS base, save user RSP
   to `PerCpu.user_rsp` (GS offset 16), load kernel RSP from `PerCpu.kernel_rsp`
   (GS offset 8).

2. **Save callee-saved registers**: Pushes RCX (user RIP), R11 (user RFLAGS),
   RBP, RBX, R12-R15 onto the kernel stack.

3. **Persist registers for blocking syscalls**: Copies the user's callee-saved
   registers plus RIP/RFLAGS into a per-CPU `SyscallSavedRegs` struct (accessed
   via `PerCpu.saved_regs_ptr` at GS offset 56). This is necessary because
   blocking syscalls like `task_wait` use `restore_kernel_context` (a longjmp),
   which abandons the kernel syscall stack.

4. **Remap to SysV C calling convention**: The Linux syscall ABI passes
   arguments in RAX, RDI, RSI, RDX, R10, R8. The stub remaps these to
   RDI, RSI, RDX, RCX, R8, R9 for the C call to `syscall_dispatch(nr, a0, a1, a2, a3, a4)`.

5. **Call `syscall_dispatch`**: The Rust dispatch function (return value in RAX).

6. **Return path**: Restores callee-saved registers. Tests bit 63 of the return
   RIP to determine the caller's privilege level:
   - **User caller** (bit 63 clear): restores user RSP, `swapgs`, `sysretq`.
   - **Kernel caller** (bit 63 set): builds an `iretq` frame and returns via
     `iretq`, since `sysretq` unconditionally loads ring 3 segments.

The `SyscallSavedRegs` struct stores user RIP, RFLAGS, RBX, RBP, and R12-R15.
It is wrapped in `SyncSavedRegs` (an `UnsafeCell` newtype) and stored as a
per-CPU `CpuLocal` static.

## Syscall dispatch

The `define_syscalls!` macro in `hadron-syscall/src/lib.rs` generates:

- **`SyscallHandler` trait** (kernel feature): one method per syscall
  (`sys_task_exit`, `sys_vnode_read`, etc.). Reserved syscalls have default
  implementations returning `-ENOSYS`. Active syscalls are required (no default body).

- **`dispatch()` function** (kernel feature): a `match` on the syscall number
  that forwards to the corresponding `SyscallHandler` method. Unknown numbers
  return `-ENOSYS`.

The kernel implements this trait on a unit struct `HadronDispatch` in
`syscall/mod.rs`, delegating each method to the appropriate handler module. A
static `DISPATCH` instance is used by the `#[no_mangle] extern "C"
syscall_dispatch` function that the assembly stub calls.

### Syscall number scheme

Each syscall group owns a range of numbers. The absolute syscall number is
`group_start + offset`. Current groups:

| Group | Range | Description |
|---|---|---|
| `task` | `0x00..0x10` | Process lifecycle |
| `handle` | `0x10..0x20` | File descriptor operations |
| `channel` | `0x20..0x30` | IPC channels (reserved) |
| `vnode` | `0x30..0x40` | Filesystem / VFS operations |
| `memory` | `0x40..0x50` | Address space management |
| `event` | `0x50..0x60` | Events, clocks, timers |
| `system` | `0xF0..0x100` | System queries and debug |

The `Syscall` and `SyscallGroup` enums provide runtime introspection (lookup by
number, name, group, argument count, reserved status).

## `UserPtr<T>` and `UserSlice` validation

All user-supplied pointers pass through validation types in `syscall/userptr.rs`
before the kernel dereferences them.

### `UserPtr<T>`

Validates a typed pointer to user memory. Construction (`UserPtr::new(addr)`)
checks:

1. **Alignment**: `addr` is aligned to `align_of::<T>()`.
2. **Overflow**: `addr + size_of::<T>()` does not overflow.
3. **Address space boundary**: the entire range `[addr, addr + size_of::<T>())`
   is below `USER_ADDR_MAX` (`0x0000_8000_0000_0000`).

Failure returns `Err(-EFAULT)`. Dereferencing (`as_ref()`) is an unsafe operation
-- the caller must ensure the memory is mapped and contains a valid `T`.

### `UserSlice`

Validates a byte range `[addr, addr + len)`. Construction checks overflow and
boundary in the same way. Provides `as_slice()` and `as_mut_slice()` (both unsafe)
for converting the validated range into Rust slices. Zero-length slices are always
valid.

### Kernel-mode bypass

The `is_kernel_caller(saved_rip)` function detects kernel-mode callers (addresses
with bit 63 set). During early boot testing, syscalls are invoked from kernel
space where `UserPtr`/`UserSlice` would reject every pointer. Handler functions
check this and skip validation for kernel addresses, falling through to direct
pointer access.

## Syscall categories

### Task (`syscall/process.rs`)

| Syscall | Number | Description |
|---|---|---|
| `task_exit` | `0x00` | Terminate the current process. Restores kernel CR3 and GS bases, stores exit status, then longjmps via `restore_kernel_context` back to the executor. |
| `task_spawn` | `0x01` | Spawn a new process from an ELF path. Validates path via `UserSlice`, reads `SpawnArg` descriptors from the parent's address space (up to 32 args, 4096 bytes total), validates UTF-8, calls `spawn_process`. Returns child PID. |
| `task_wait` | `0x02` | Block until a child exits. Sets `TRAP_WAIT` reason, longjmps to `process_task` which handles the async wait. Never returns to the caller directly. |
| `task_info` | `0x05` | Returns the current process PID. |
| `task_kill` | `0x03` | Reserved (Phase 11). |
| `task_detach` | `0x04` | Reserved (Phase 11). |

### Handle (`syscall/vfs.rs`)

| Syscall | Number | Description |
|---|---|---|
| `handle_close` | `0x10` | Close a file descriptor via the process fd table. |
| `handle_dup` | `0x11` | Duplicate a file descriptor with dup2 semantics (close target if open). |
| `handle_pipe` | `0x13` | Create a pipe. Writes `[read_fd, write_fd]` to the user buffer. |
| `handle_info` | `0x12` | Reserved (Phase 11). |

### VFS / Vnodes (`syscall/vfs.rs`)

| Syscall | Number | Description |
|---|---|---|
| `vnode_open` | `0x30` | Resolve a path via the VFS, allocate an fd with the given `OpenFlags`. |
| `vnode_read` | `0x31` | Read from an fd. Uses `try_poll_immediate` -- if the I/O would block (e.g. pipe), triggers `TRAP_IO` for async handling. Updates file offset on success. |
| `vnode_write` | `0x32` | Write to an fd. Same async trap logic as read. |
| `vnode_stat` | `0x33` | Write a `StatInfo` struct to the user buffer (inode type, size, permissions). |
| `vnode_readdir` | `0x34` | Read directory entries as a `DirEntryInfo` array. Returns entry count. |
| `vnode_unlink` | `0x35` | Reserved (Phase 10). |

### Memory (`syscall/memory.rs`)

| Syscall | Number | Description |
|---|---|---|
| `mem_map` | `0x40` | Stub, returns `-ENOSYS`. |
| `mem_unmap` | `0x41` | Stub, returns `-ENOSYS`. |
| `mem_protect` | `0x42` | Reserved (Phase 9). |
| `mem_create_shared` | `0x43` | Reserved (Phase 11). |
| `mem_map_shared` | `0x44` | Reserved (Phase 11). |

### Events and time (`syscall/time.rs`)

| Syscall | Number | Description |
|---|---|---|
| `clock_gettime` | `0x54` | Returns boot-relative monotonic time as a `Timespec` (u64 seconds + u64 nanoseconds). Only `CLOCK_MONOTONIC` (0) is supported. Backed by the HPET `boot_nanos()` clock source. |
| `event_create` | `0x50` | Reserved (Phase 11). |
| `event_signal` | `0x51` | Reserved (Phase 11). |
| `event_wait` | `0x52` | Reserved (Phase 11). |
| `event_wait_many` | `0x53` | Reserved (Phase 11). |
| `timer_create` | `0x55` | Reserved (Phase 11). |

### System services (`syscall/query.rs`, `syscall/io.rs`)

| Syscall | Number | Description |
|---|---|---|
| `query` | `0xF0` | Return typed `#[repr(C)]` system information structs. Topics: `QUERY_MEMORY` (physical RAM stats as `MemoryInfo`), `QUERY_UPTIME` (nanoseconds since boot as `UptimeInfo`), `QUERY_KERNEL_VERSION` (version + name as `KernelVersionInfo`). |
| `debug_log` | `0xF1` | Write a UTF-8 message to the kernel serial console via `kprint!`. Returns byte count. |

## Blocking syscalls and trap mechanism

Some syscalls cannot complete synchronously. Hadron handles these using a
**trap-and-longjmp** mechanism rather than blocking the kernel thread:

1. The syscall handler sets up parameters in process-global state (e.g.
   `set_wait_params`, `set_io_params`) and sets a trap reason (`TRAP_EXIT`,
   `TRAP_WAIT`, `TRAP_IO`).

2. It restores the kernel CR3 and GS bases (undoing the user address space switch).

3. It calls `restore_kernel_context(saved_rsp)`, which longjmps back to the
   `process_task` on the async executor. The kernel syscall stack frame is abandoned.

4. The `process_task` reads the trap reason and parameters, performs the async
   operation (e.g. awaiting a child exit future, retrying blocked I/O), then
   re-enters userspace with the result placed in RAX.

The per-CPU `SyscallSavedRegs` ensures user register state survives the longjmp.

## Error handling

All syscalls return `isize`. The convention is:

- **Non-negative values**: success. The meaning is syscall-specific (byte count, fd number, PID, or 0 for void operations).
- **Negative values**: negated POSIX-compatible error code.

The error codes are defined in the `errors` block of `define_syscalls!`:

| Constant | Value | Meaning |
|---|---|---|
| `ENOENT` | 2 | No such file or directory |
| `EIO` | 5 | I/O error |
| `EBADF` | 9 | Bad file descriptor |
| `EACCES` | 13 | Permission denied |
| `EFAULT` | 14 | Bad address (failed `UserPtr`/`UserSlice` validation) |
| `EEXIST` | 17 | File exists |
| `ENOTDIR` | 20 | Not a directory |
| `EISDIR` | 21 | Is a directory |
| `EINVAL` | 22 | Invalid argument |
| `ENOSYS` | 38 | Function not implemented |
| `ELOOP` | 40 | Too many symbolic link levels |

Kernel VFS errors are converted to errno values via `.to_errno()` on the internal
error types.

## Shared types

The `types` block in `define_syscalls!` generates `#[repr(C)]` structs shared
between kernel and userspace:

- **`Timespec`** -- `{ tv_sec: u64, tv_nsec: u64 }`. Uses unsigned fields since Hadron only supports monotonic boot-relative time.
- **`MemoryInfo`** -- `{ total_bytes: u64, free_bytes: u64, used_bytes: u64 }`.
- **`UptimeInfo`** -- `{ uptime_ns: u64 }`.
- **`KernelVersionInfo`** -- `{ major: u16, minor: u16, patch: u16, _pad: u16, name: [u8; 32] }`.
- **`StatInfo`** -- `{ inode_type: u8, _pad: [u8; 3], size: u64, permissions: u32 }`.
- **`SpawnArg`** -- `{ ptr: usize, len: usize }`. Argument descriptor for `task_spawn`.
- **`DirEntryInfo`** -- `{ inode_type: u8, name_len: u8, _pad: [u8; 2], name: [u8; 60] }`.

## Userspace ABI

When compiled with the `userspace` feature, `hadron-syscall` generates:

- **`raw::syscall0` through `raw::syscall5`**: inline assembly functions that
  place the syscall number in RAX, arguments in RDI/RSI/RDX/R10/R8, execute
  `SYSCALL`, and return RAX. All caller-saved registers are declared as clobbered.

- **`wrappers::sys_*`**: typed wrapper functions for each non-reserved syscall
  that call the appropriate `raw::syscallN` with the correct constant and
  argument count. These are what userspace programs call.

## Testing

Integration tests in `kernel/hadron-kernel/tests/syscall_test.rs` invoke
syscalls from kernel space using inline `syscall` instructions. Since these run
before userspace exists, the addresses have bit 63 set, and handlers use
`is_kernel_caller` to skip user-pointer validation. Tests cover:

- `task_info` returns a non-negative value.
- Unknown syscall numbers return `-ENOSYS`.
- `debug_log` returns the message length.
- `clock_gettime` with valid and invalid clock IDs.

## Reserved syscalls

Syscalls annotated with `#[reserved(phase = N)]` in the DSL are placeholders
for future phases. They get `SYS_*` constants and `Syscall` enum variants now,
but their `SyscallHandler` trait methods have default implementations returning
`-ENOSYS`. No userspace wrappers are generated for reserved syscalls.
