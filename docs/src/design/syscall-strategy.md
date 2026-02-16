# Syscall Strategy

> **Note:** The three-layer architecture described here (unstable ABI, stable library, vDSO) remains valid, but the syscall *set* has been redesigned around a native handle-based interface. See [Task-Centric OS Design](task-centric-design.md) for the current syscall table. The Linux-compatible numbering described below may still be used for early bootstrapping and testing.

Hadron uses a three-layer syscall strategy designed to give the kernel maximum freedom to evolve while providing userspace applications a stable interface.

## The Three Layers

```
┌─────────────────────────────────────────────────┐
│              User Application                    │
│         (calls stable library API)               │
├─────────────────────────────────────────────────┤
│         Stable Userspace Library                 │
│    (hadron-libc / libhadron, separate project)   │
│    Translates stable API → current kernel ABI    │
├─────────────────────────────────────────────────┤
│              vDSO (Phase 15)                     │
│    Fast-path read-only ops without syscall       │
│    (clock_gettime, gettimeofday, getcpu)         │
├─────────────────────────────────────────────────┤
│         Unstable Internal Kernel ABI             │
│    (SYSCALL instruction, register convention)    │
│    Can change freely between kernel versions     │
└─────────────────────────────────────────────────┘
```

### Layer 1: Unstable Internal ABI

The raw kernel syscall interface is **explicitly unstable**. This means:

- Syscall numbers can be renumbered
- Argument layouts can change
- New syscalls can replace old ones
- Struct layouts passed via pointers can change

**Why?** An unstable internal ABI gives us freedom to:
- Redesign syscall interfaces as we learn what works
- Merge or split syscalls without worrying about backwards compatibility
- Optimize hot paths by changing argument order or encoding
- Add new capabilities without being constrained by legacy decisions

**Initial approach**: Use Linux syscall numbers to start. This lets us test with existing statically-linked Linux binaries and tools before we have our own userspace library. Once the userspace library exists, we can diverge.

### Layer 2: Stable Userspace Library

A userspace shared library (future separate project) provides the stable API:

```rust
// Stable API — never changes signature
pub fn open(path: &str, flags: OpenFlags) -> Result<Fd, Errno> {
    // Translates to current kernel ABI
    // If kernel v2 renumbers SYS_OPEN from 2 to 42, only this
    // library needs updating — applications don't change.
    unsafe { syscall3(SYS_OPEN, path.as_ptr() as usize, flags.bits(), 0) }
}
```

Benefits:
- **Applications link against the library**, not the raw kernel ABI
- **Single update point**: When the kernel ABI changes, update the library, not every application
- **Version negotiation**: Library can detect kernel version and use the appropriate ABI
- **Error translation**: Convert kernel error codes to stable errno values

### Layer 3: vDSO

The vDSO (virtual Dynamic Shared Object) is a kernel-provided shared library mapped into every process. It handles hot-path, read-only operations without a syscall:

| vDSO Function | What It Does | Performance |
|---------------|-------------|-------------|
| `__vdso_clock_gettime` | Read wall/monotonic clock | ~20 ns (vs ~200 ns syscall) |
| `__vdso_gettimeofday` | Legacy time interface | ~20 ns |
| `__vdso_getcpu` | Current CPU ID | ~5 ns |

The vDSO reads from a shared VVAR page that the kernel timer interrupt keeps updated. No mode switch required.

## Syscall Register Convention

Following the System V AMD64 ABI (same as Linux):

| Register | Purpose |
|----------|---------|
| `rax` | Syscall number |
| `rdi` | Argument 1 |
| `rsi` | Argument 2 |
| `rdx` | Argument 3 |
| `r10` | Argument 4 (not `rcx` — `syscall` clobbers it) |
| `r8` | Argument 5 |
| `r9` | Argument 6 |
| `rax` | Return value (negative = `-errno`) |

The `syscall` instruction saves RIP in `rcx` and RFLAGS in `r11`, which is why argument 4 uses `r10` instead of `rcx`.

## Initial Syscall Set

Phase 7 implements these first, using Linux numbering:

| Number | Name | Category | Description |
|--------|------|----------|-------------|
| 0 | `read` | I/O | Read from file descriptor |
| 1 | `write` | I/O | Write to file descriptor |
| 2 | `open` | I/O | Open a file |
| 3 | `close` | I/O | Close file descriptor |
| 9 | `mmap` | Memory | Map memory |
| 11 | `munmap` | Memory | Unmap memory |
| 12 | `brk` | Memory | Change data segment size |
| 39 | `getpid` | Process | Get process ID |
| 60 | `exit` | Process | Terminate process |
| 228 | `clock_gettime` | Time | Get clock value |

## Growth Path

The syscall set grows incrementally with each phase:

| Phase | New Syscalls |
|-------|-------------|
| 7 | `read`, `write`, `open`, `close`, `mmap`, `munmap`, `brk`, `exit`, `getpid` |
| 8 | `stat`, `fstat`, `lseek`, `readdir`, `mkdir`, `unlink` |
| 9 | (no new syscalls — uses existing ones) |
| 11 | `pipe`, `fork`, `execve`, `waitpid`, `kill`, `sigaction`, `dup2` |
| 13 | `socket`, `bind`, `listen`, `accept`, `connect`, `send`, `recv` |
| 15 | `clock_gettime` via vDSO (replaces syscall path) |

Target: ~50 syscalls by Phase 11, growing to ~200 as features are added.

## Why Not Stable From Day One?

Starting with an unstable ABI is the pragmatic choice for a new kernel:

1. **We don't know the right design yet**. Early syscall interfaces will have mistakes.
2. **No existing userspace to break**. There's no backwards-compatibility burden until applications exist.
3. **Linux numbers bootstrap testing**. Using Linux numbers lets us run existing test binaries immediately.
4. **The stable library can be added later** without changing any kernel code — it's a pure userspace translation layer.

When the kernel matures and the userspace library exists, the internal ABI can diverge from Linux while maintaining application compatibility through the library.
