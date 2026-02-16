# Phase 15: vDSO & Performance

Minor updates from the original plan. Adds a futex syscall for efficient userspace synchronization primitives.

## Goal

Implement a virtual Dynamic Shared Object (vDSO) mapped into every process for fast `clock_gettime` without a syscall. Maintain a shared VVAR data page updated by the kernel timer interrupt, read by userspace via a seqlock protocol with TSC interpolation. Add a `sys_futex` syscall to support userspace mutexes and condition variables.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/vdso/mod.rs` | vDSO ELF image generation and per-process mapping |
| `hadron-kernel/src/vdso/data.rs` | VVAR page layout, seqlock update logic |
| `hadron-kernel/src/vdso/code.rs` | vDSO function implementations (compiled into ELF) |
| `hadron-kernel/src/syscall/futex.rs` | `sys_futex` implementation |
| `hadron-core/src/arch/x86_64/tsc.rs` | TSC reading and calibration constants |

## Key Design

### vDSO Overview

The vDSO is a small shared library that the kernel maps into every process's address space. It contains functions like `__vdso_clock_gettime` that read time data directly from a shared page (VVAR) instead of issuing a syscall. This eliminates the cost of ring transitions for the most frequently called time functions.

The mapping consists of two regions:
- **VVAR page**: read-only data page containing timestamps and TSC parameters, updated by the kernel on each timer interrupt.
- **vDSO code pages**: read+execute pages containing the vDSO function code.

### VVAR Page and Seqlock Protocol

The kernel writes time data to the VVAR page on every timer interrupt. Userspace reads it without any locks, using a seqlock protocol to detect torn reads.

**Kernel side** (called from timer interrupt handler):

```rust
pub fn update_vvar(vvar: &VvarData) {
    // Increment seq to odd: signals write in progress
    let seq = vvar.seq.load(Ordering::Relaxed);
    vvar.seq.store(seq + 1, Ordering::Release);

    // Write updated time values
    vvar.wall_time_sec = current_wall_sec;
    vvar.wall_time_nsec = current_wall_nsec;
    vvar.monotonic_sec = current_mono_sec;
    vvar.monotonic_nsec = current_mono_nsec;
    vvar.tsc_timestamp = rdtsc();
    vvar.tsc_mult = calibrated_mult;
    vvar.tsc_shift = calibrated_shift;

    // Increment seq to even: signals write complete
    compiler_fence(Ordering::Release);
    vvar.seq.store(seq + 2, Ordering::Release);
}
```

**Userspace side** (runs in vDSO, no syscall):

```rust
pub fn __vdso_clock_gettime(clock_id: u32, tp: &mut Timespec) -> i32 {
    let vvar = unsafe { &*(VVAR_ADDR as *const VvarData) };

    loop {
        let seq1 = vvar.seq.load(Ordering::Acquire);
        if seq1 & 1 != 0 {
            core::hint::spin_loop();
            continue; // Write in progress, retry
        }

        // Read snapshot
        let wall_sec = vvar.wall_time_sec;
        let wall_nsec = vvar.wall_time_nsec;
        let tsc_base = vvar.tsc_timestamp;
        let mult = vvar.tsc_mult;
        let shift = vvar.tsc_shift;

        compiler_fence(Ordering::Acquire);
        let seq2 = vvar.seq.load(Ordering::Acquire);
        if seq1 != seq2 {
            continue; // Data changed during read, retry
        }

        // Interpolate current time using TSC
        let tsc_now = rdtsc();
        let tsc_delta = tsc_now - tsc_base;
        let nsec_delta = (tsc_delta as u128 * mult as u128) >> shift;

        tp.tv_sec = wall_sec as i64 + (nsec_delta / 1_000_000_000) as i64;
        tp.tv_nsec = wall_nsec as i64 + (nsec_delta % 1_000_000_000) as i64;
        if tp.tv_nsec >= 1_000_000_000 {
            tp.tv_sec += 1;
            tp.tv_nsec -= 1_000_000_000;
        }

        return 0;
    }
}
```

### TSC Interpolation

The TSC (Time Stamp Counter) increments at a fixed rate on modern CPUs. The VVAR page stores precomputed multiplication and shift parameters calibrated during boot:

```
nsec_since_last_update = (rdtsc() - vvar.tsc_timestamp) * vvar.tsc_mult >> vvar.tsc_shift
```

This avoids division entirely. The kernel calibrates `tsc_mult` and `tsc_shift` against the APIC timer or HPET during Phase 5 boot.

### vDSO Mapping

```rust
/// Map the vDSO and VVAR page into a process address space.
pub fn map_vdso(address_space: &mut AddressSpace) -> Result<VirtAddr, MapError> {
    // Map VVAR page: read-only + user-accessible
    address_space.mapper().map(
        VVAR_USER_ADDR,
        GLOBAL_VVAR_FRAME,
        PageTableFlags::PRESENT | PageTableFlags::USER,
        &mut allocator,
    )?;

    // Map vDSO code pages: read + execute + user-accessible
    for (i, frame) in VDSO_CODE_FRAMES.iter().enumerate() {
        address_space.mapper().map(
            VDSO_USER_ADDR + (i * PAGE_SIZE) as u64,
            *frame,
            PageTableFlags::PRESENT | PageTableFlags::USER, // No WRITABLE
            &mut allocator,
        )?;
    }

    Ok(VDSO_USER_ADDR)
}
```

### Futex Syscall

The futex ("fast userspace mutex") syscall provides the kernel-side sleeping mechanism for userspace synchronization primitives. Userspace mutexes use atomic operations in the uncontended fast path and fall back to `sys_futex` only when contention requires sleeping.

```rust
/// sys_futex(addr, op, val, timeout)
///
/// FUTEX_WAIT: If *addr == val, sleep on a WaitQueue keyed by addr.
///             Returns when woken or on timeout.
/// FUTEX_WAKE: Wake up to val waiters sleeping on addr.
pub fn sys_futex(
    addr: UserPtr<AtomicU32>,
    op: u32,
    val: u32,
    timeout: Option<Duration>,
) -> Result<usize, SyscallError> {
    match op {
        FUTEX_WAIT => {
            let current = addr.read_atomic(Ordering::SeqCst)?;
            if current != val {
                return Err(SyscallError::Again); // Value changed, retry in userspace
            }
            let wq = futex_wait_queue(addr.as_usize());
            match timeout {
                Some(t) => wq.wait_timeout(t),
                None => wq.wait(),
            }
            Ok(0)
        }
        FUTEX_WAKE => {
            let wq = futex_wait_queue(addr.as_usize());
            let woken = wq.wake_n(val as usize);
            Ok(woken)
        }
        _ => Err(SyscallError::InvalidArgument),
    }
}
```

The futex wait queue table is a hash map from user virtual addresses to `WaitQueue` instances. This allows any aligned 32-bit word in user memory to serve as a futex.

### Supported vDSO Functions

| Function | Description | Fallback Syscall |
|----------|-------------|------------------|
| `__vdso_clock_gettime` | Wall/monotonic time via VVAR + TSC | `SYS_clock_gettime` |
| `__vdso_gettimeofday` | Legacy time interface | `SYS_gettimeofday` |
| `__vdso_getcpu` | Current CPU ID from VVAR | `SYS_getcpu` |

## Key Data Structures

### VVAR Page

```rust
/// Data page shared between kernel and userspace.
/// Updated by the kernel timer interrupt; read by vDSO functions.
#[repr(C)]
pub struct VvarData {
    /// Seqlock counter. Odd = write in progress, even = consistent.
    pub seq: AtomicU32,

    /// Wall clock time at last kernel update.
    pub wall_time_sec: u64,
    pub wall_time_nsec: u32,

    /// Monotonic time at last kernel update.
    pub monotonic_sec: u64,
    pub monotonic_nsec: u32,

    /// TSC value captured at last kernel update.
    pub tsc_timestamp: u64,

    /// Precomputed TSC-to-nanosecond conversion parameters.
    /// nsec = (tsc_now - tsc_timestamp) * tsc_mult >> tsc_shift
    pub tsc_mult: u32,
    pub tsc_shift: u32,

    /// CPU ID (for __vdso_getcpu).
    pub cpu_id: u32,
}
```

### Futex Hash Table

```rust
/// Global futex wait queue table.
/// Maps user virtual addresses to wait queues.
pub struct FutexTable {
    buckets: [SpinLock<FutexBucket>; FUTEX_HASH_SIZE],
}

struct FutexBucket {
    waiters: Vec<FutexWaiter>,
}

struct FutexWaiter {
    addr: usize,
    waker: Waker,
}
```

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| TSC reading (`rdtsc`) | Frame | Inline assembly |
| VVAR physical frame allocation | Frame | Physical memory management |
| `VvarData` struct definition | Frame | Shared kernel/userspace memory layout |
| Seqlock update in timer interrupt | Service | Called from timer handler using safe VVAR reference |
| vDSO ELF image generation | Service | Pure byte construction, no hardware access |
| vDSO/VVAR mapping into process | Service | Uses safe address space mapping APIs |
| `sys_futex` implementation | Service | WaitQueue operations on validated user pointers |
| Futex hash table | Service | Data structure management, no unsafe |

## Milestone

**Verification**: Benchmark `clock_gettime()` via vDSO path versus syscall path:

```
Benchmark: clock_gettime (10,000 iterations)
  vDSO path:    ~200 us total (~20 ns/call)
  Syscall path: ~2000 us total (~200 ns/call)
  Speedup: ~10x
```

The vDSO path avoids: ring 3 to ring 0 mode switch, `swapgs`, stack switch, register save/restore, and the return transition. Only a few memory reads and a `rdtsc` instruction are needed.

Futex verification:

```
futex_test: spawning 4 threads contending on mutex
futex_test: all threads completed, counter = 40000 (expected 40000)
futex_test: PASS
```

## Dependencies

- **Phase 7**: Syscall interface (`sys_futex` is a new syscall; vDSO is an optimization over existing syscalls)
- **Phase 9**: Userspace (vDSO is mapped into user process address spaces)
- **Phase 5**: Timer subsystem (TSC calibration, timer interrupt drives VVAR updates)
