# Synchronization Primitives

Hadron's synchronization layer lives in the `hadron-core` crate under `kernel/core/src/sync/`. Every primitive is generic over a `Backend` trait, which allows the same lock implementations to run under `loom` or `shuttle` for formal concurrency verification, and to execute on real hardware using the `CoreBackend` type alias. Each section below describes the contract, the internal state machine, and the appropriate usage context for one primitive.

## Backend Trait System

All primitives are parameterized by either `Backend` or `IrqBackend`:

```rust
// Production type aliases resolve to CoreBackend
pub type SpinLock<T>   = SpinLockInner<T, CoreBackend>;
pub type IrqSpinLock<T> = IrqSpinLockInner<T, CoreBackend>;
pub type Mutex<T>      = MutexInner<T, CoreBackend>;
pub type RwLock<T>     = RwLockInner<T, CoreBackend>;
pub type SeqLock<T>    = SeqLockInner<T, CoreBackend>;
pub type Semaphore<T>  = SemaphoreInner<T, CoreBackend>;
```

The `Backend` trait abstracts over atomic operations (`AtomicBool`, `AtomicU32`, `AtomicUsize`) and `UnsafeCell`. `IrqBackend` extends `Backend` with interrupt enable/disable primitives. On kernel targets these map to `cli`/`sti` (or `pushfq`/`popfq`). On host-test targets they are no-ops.

This design keeps `cfg(loom)` and `cfg(shuttle)` contained entirely within `backend.rs` and `loom_mock.rs`. All higher-level code is backend-agnostic.

```mermaid
graph LR
    SpinLock --> CoreBackend
    SpinLock --> LoomBackend
    SpinLock --> ShuttleBackend
    IrqSpinLock --> CoreBackend
    Mutex --> CoreBackend
    Mutex --> LoomBackend
    CoreBackend --> "real atomics + UnsafeCell"
    LoomBackend --> "loom atomics (model checking)"
    ShuttleBackend --> "shuttle atomics (randomized)"
```

## SpinLock

`SpinLock<T>` is a test-and-test-and-set (TTAS) mutual exclusion lock. It spins on a load (test) before attempting a compare-exchange (set), reducing cache-line bouncing under contention.

### Constructors

```rust
// Unnamed lock — lowest overhead when lockdep is disabled
let lock = SpinLock::new(value);

// Named lock — name appears in lockdep diagnostics
let lock = SpinLock::named("my_lock", value);

// Named + ordered — lockdep enforces acquisition ordering
let lock = SpinLock::leveled("my_lock", LEVEL_MM, value);
```

When compiled without `cfg(hadron_lockdep)`, the `name` and `level` fields are zero-sized and the constructor arguments are dropped at compile time via `let _ = (name, level)`.

### Acquisition Protocol

```
acquire:
  loop {
    while locked.load(Relaxed) == true { spin }
    if locked.compare_exchange(false, true, Acquire, Relaxed).is_ok() { break }
  }

release:
  locked.store(false, Release)
```

The `Acquire` barrier on successful CAS synchronizes with the `Release` on unlock, establishing a happens-before relationship between the critical sections.

### Usage Rules

- Do not hold a `SpinLock` across an `.await` point. Awaiting while spinning will deadlock if the executor tries to run another task on the same CPU.
- Do not hold a `SpinLock` across a function that may re-acquire the same lock (recursive locking is undefined behavior — the lock has no owner field).
- Prefer `IrqSpinLock` for any lock shared between interrupt handlers and normal kernel code.

## IrqSpinLock

`IrqSpinLock<T>` saves the current interrupt-enable state (`pushfq`), disables interrupts (`cli`), then acquires the inner spinlock. On release it restores the saved state (`popfq`).

### Why Interrupt Disabling Is Necessary

If a normal `SpinLock` is held by a kernel thread and an interrupt fires on the same CPU, and the interrupt handler also attempts to acquire the same lock, the CPU will spin forever: the lock holder is preempted and cannot release the lock.

`IrqSpinLock` eliminates this hazard entirely by ensuring no interrupt can fire between acquisition and release.

### Nesting Depth Tracking

When `cfg(hadron_lock_debug)` is active, a per-CPU `CpuLocal<AtomicU32>` counter tracks the nesting depth of `IrqSpinLock` acquisitions. If the depth exceeds 3 and `cfg(hadron_lockdep)` is also active, a warning is emitted through the lockdep reporting callback. Deep nesting widens the interrupt-disabled window and increases worst-case interrupt latency.

### Usage Rules

- Use for any lock accessed from both interrupt context and process context on the same CPU.
- Never call `Mutex::lock().await` inside an `IrqSpinLock` critical section — the task cannot yield while interrupts are disabled.
- The lock is not reentrant. Re-acquiring on the same CPU will deadlock.

## Mutex

`Mutex<T>` is an async-aware mutual exclusion lock. When the lock is contended, the calling task registers its `Waker` in the lock's internal `WaitQueue` and returns `Poll::Pending`, yielding the CPU to other tasks. When the lock holder releases it, the oldest waiter is woken.

### Internal State

```
locked: AtomicBool   -- 0 = free, 1 = held
waiters: WaitQueue   -- ordered list of Waker values
```

The lock() future state machine:

```
Poll::Ready if locked CAS false->true succeeds
Poll::Pending if CAS fails:
  - register waker in waiters queue
  - re-try CAS (handles wakeup-before-register race)
  - if still locked, return Pending
```

### Integration with the Executor

`Mutex::lock()` returns a `MutexLockFuture` that implements `Future<Output = MutexGuard>`. Kernel code uses it as:

```rust
let guard = mutex.lock().await;
// guard released on drop
```

The waker mechanism bridges the lock back to the per-CPU executor's ready queue: when a waiter is woken, its task is moved from a wait list into the executor's run queue and will be polled on the next scheduler iteration.

### Usage Rules

- Safe to hold across `.await` points (the task yields, not the CPU).
- Do not use from interrupt context (interrupt handlers cannot await).
- Prefer `Mutex` over `SpinLock` when the critical section may block or when contention is expected to be high.

## RwLock

`RwLock<T>` allows multiple concurrent readers or a single exclusive writer. The state is packed into a single `AtomicUsize`:

- `0` means unlocked.
- Positive values count active readers.
- A sentinel value (`usize::MAX`) represents a writer holding the lock.

Reader acquisition increments the count if no writer holds the lock. Writer acquisition performs a CAS from zero to the sentinel. Write attempts spin-wait until the reader count reaches zero.

### Usage Guidelines

`RwLock` is appropriate for data that is read frequently but written rarely, such as routing tables, device registries, or mount tables. For data that changes at every tick (e.g., per-CPU counters), use `SeqLock` instead.

## SeqLock

`SeqLock<T>` is optimized for read-mostly data that must be updated atomically without blocking readers. The sequence number state machine uses parity:

- Even sequence number: data is consistent (no write in progress).
- Odd sequence number: a write is in progress.

### Read Protocol

```
loop {
  seq1 = seq.load(Acquire)
  if seq1 is odd { spin }
  copy = data.read()
  seq2 = seq.load(Acquire)
  if seq1 == seq2 { return copy }  // retry if write happened during copy
}
```

### Write Protocol

```
seq.fetch_add(1, Release)  // make odd (begin write)
data.write(new_value)
seq.fetch_add(1, Release)  // make even (end write)
```

### Constraints

`T` must be `Copy` because readers take a bitwise copy of the data. If `T` contains pointers to heap-allocated data, a concurrent write could invalidate the copy before it is used. `SeqLock` is therefore appropriate only for small, self-contained values such as timestamps, clock ticks, or hardware configuration registers.

Kernel uses include: the HPET tick counter, per-CPU load statistics, and the system-wide monotonic clock.

## Semaphore

`Semaphore` is a counting semaphore backed by an async `WaitQueue`. The internal count is stored in an `AtomicUsize`. Acquiring decrements the count; if the count would go negative, the task yields and waits until another caller releases a permit.

```rust
// Create a semaphore allowing 4 concurrent holders
let sem = Semaphore::new(4);

// Async acquire — returns a SemaphorePermit on success
let permit = sem.acquire().await;
// permit.drop() releases the permit and wakes one waiter
```

`SemaphorePermit` is an RAII guard that releases the semaphore count on drop. This ensures permits are always returned even when the holder returns early due to an error.

## HeapWaitQueue

`HeapWaitQueue` is a kernel wait queue for blocking I/O and object-signal subscription. Unlike the internal `WaitQueue` used by `Mutex` and `Semaphore` (which is bounded and spinlock-protected), `HeapWaitQueue` uses a heap-allocated `Vec<Waker>` and is suitable for objects that may have an unbounded number of concurrent waiters (e.g., a Channel with many readers).

```rust
let wq = HeapWaitQueue::new();

// Subscriber side (async)
wq.wait_until(|| condition_met()).await;

// Notifier side (from any context)
wq.wake_all();
wq.wake_one();
```

`HeapWaitFuture` is the `Future` type returned by `wait_until`. It registers the waker on first poll and unregisters it if the future is dropped before completion, preventing spurious wakeups from accumulating.

## Lockdep: Compile-Assisted Lock Ordering

Lockdep is a runtime lock dependency tracker gated behind `cfg(hadron_lockdep)`. It detects potential deadlocks by recording a directed edge "lock class A was held when lock class B was acquired" in a dependency graph. On each new edge, it runs depth-first-search (DFS) cycle detection. A cycle means there exists a code path that acquires A then B and another path that acquires B then A — a potential deadlock even if it has not manifested yet.

### Capacity

| Resource | Limit |
|----------|-------|
| Lock classes | 256 |
| Nesting depth per CPU | 32 |
| Dependency graph edges | 1024 |

### Lock Levels

Each `SpinLock` and `IrqSpinLock` can be assigned a numeric level via `leveled()`. The invariant is:

> A lock at level N may only be acquired while holding locks at level less than or equal to N.

Level 0 means "unordered" — no ordering check is performed. Assigning levels to all locks in a subsystem makes the ordering policy explicit and machine-verifiable.

Example levels used in the kernel (from `timer.rs`):

```rust
static SLEEP_QUEUE: IrqSpinLock<BinaryHeap<Reverse<SleepEntry>>> =
    IrqSpinLock::leveled("SLEEP_QUEUE", 12, BinaryHeap::new());
```

### IRQ-Safety Validation

Lockdep also tracks whether each lock class has been acquired in IRQ context (inside an `IrqSpinLock` critical section). If a lock class is first used outside IRQ context and later used inside, or vice versa with inconsistent nesting, lockdep emits a warning. This catches the scenario where a developer adds a new acquisition site without realizing the lock was already used in an interrupt handler.

### Warning-Only Mode

With `cfg(hadron_lockdep_warn)`, violations are logged but do not cause a kernel panic. This is useful during bringup of new subsystems where all orderings may not yet be settled.

### Lock Contention Statistics

With `cfg(hadron_lock_stat)`, lockdep records per-class acquisition counts, contention counts, and hold/wait time histograms. These are exposed through the `query` syscall's system topic.

### Lockdep Violation Reporting Callback

`hadron-core` cannot directly call the kernel's logging subsystem (to avoid circular dependencies). Instead, the kernel registers a writer function during early init:

```rust
// In kernel early init
hadron_core::sync::lockdep::set_reporter(|args| kpanic!("{}", args));
```

In warning-only mode the callback calls the kernel logger instead of panicking.

## CpuLocal Storage

`CpuLocal<T>` provides per-CPU storage by wrapping a fixed-size `[T; MAX_CPUS]` array. On kernel targets, the current CPU index is read from the GS segment base at offset 24 (the `cpu_id` field of `PerCpu`). On host-test targets, index 0 is always used.

```rust
static MY_COUNTER: CpuLocal<AtomicU32> =
    CpuLocal::new([const { AtomicU32::new(0) }; MAX_CPUS]);

// Access current CPU's slot
MY_COUNTER.get().fetch_add(1, Ordering::Relaxed);

// Access a specific CPU's slot (for cross-CPU inspection)
MY_COUNTER.get_for(cpu_id).load(Ordering::Acquire);
```

`MAX_CPUS` is 256, matching the Kconfig upper bound. `CpuLocal<T>` implements `Send + Sync` when `T: Send`, because each CPU accesses only its own slot and there is no sharing across CPUs for the types it is used with.

### Fallback on Uninitialized GS

During early AP boot, before the GS base is written, `current_cpu_id()` may return garbage. `CpuLocal::get()` handles this by falling back to slot 0 when the returned index exceeds `MAX_CPUS`. This is safe for the atomic counters that are the primary users of `CpuLocal` during early boot.

## UnsafeCell Wrappers

`hadron-core` provides a thin `sync::cell` module with `SyncUnsafeCell<T>` — an `UnsafeCell<T>` that implements `Send + Sync` unconditionally. This is used inside lock implementations where the lock protocol itself guarantees exclusive access. It must never be used outside a correct synchronization protocol.

## AtomicPtr Wrapper

`sync::atomic::AtomicPtr<T>` is a thin wrapper around `core::sync::atomic::AtomicPtr<T>` that provides ergonomic `load`/`store`/`compare_exchange` methods with correct lifetime handling. It is used in `AtomicFn` (for registering architecture-specific callbacks such as TLB flush) and in the lockdep dependency graph's pointer arrays.

## Concurrency Testing

### Loom

`just loom` compiles the sync module with `cfg(loom)` and substitutes `LoomBackend` for `CoreBackend`. Loom exhaustively explores all interleavings of atomic operations across a bounded execution. The sync test suite verifies:

- Lock protocol correctness (acquire/release ordering under all interleavings).
- Atomic state machine transitions (e.g., `LazyLock` initialization races).
- Waker management (no lost wakeups in `Mutex` and `Semaphore`).
- Absence of data races on the protected data.

Loom does not model hardware interrupts or CPU-local isolation. Those properties are covered by `just test --kernel-only` integration tests running on emulated hardware in QEMU.

### Shuttle

`cfg(shuttle)` enables `ShuttleBackend` for randomized concurrency testing. Unlike loom's exhaustive search, shuttle uses a probabilistic scheduler that discovers bugs quickly in large codebases where state-space explosion makes exhaustive search impractical.

### Miri

`just miri` runs the host-testable portions of `hadron-core` under Miri, detecting undefined behavior in unsafe code (out-of-bounds memory access, use-after-free, incorrect use of `UnsafeCell`, invalid pointer provenance).
