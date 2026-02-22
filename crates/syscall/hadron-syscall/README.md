# hadron-syscall

Single source of truth for all Hadron syscall definitions, shared between the
kernel and userspace. This `no_std` crate uses the `define_syscalls!` proc macro
to generate syscall number constants, error codes, `#[repr(C)]` data structures,
named constants, dispatch enums, and -- depending on the active feature flag --
either kernel-side handler traits or userspace-side raw `syscall` assembly stubs
and typed wrapper functions, all from one declarative definition.

## Features

- **Unified definition** -- a single DSL in `lib.rs` defines errors, types,
  constants, and grouped syscalls; the proc macro generates everything else.
- **Grouped syscall numbering** -- syscalls are organized into groups (task,
  handle, channel, vnode, memory, event, system) with non-overlapping number
  ranges.
- **Shared `#[repr(C)]` types** -- structures like `Timespec`, `StatInfo`,
  `SpawnInfo`, and `DirEntryInfo` are generated once and used by both kernel
  and userspace.
- **Feature-gated code generation** -- `kernel` feature emits a
  `SyscallHandler` trait and `dispatch()` function; `userspace` feature emits
  raw `syscallN` inline-asm stubs and safe typed wrappers.
- **Reserved syscall slots** -- `#[reserved(phase = N)]` marks syscalls planned
  for future phases, reserving their numbers while preventing premature use.
- **Validation** -- the proc macro validates that syscall offsets fall within
  their group's declared range and that no two syscalls collide.
