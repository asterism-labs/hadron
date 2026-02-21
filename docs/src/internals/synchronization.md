# Synchronization Primitives

Hadron provides a layered set of synchronization primitives in the
`hadron_kernel::sync` module (`kernel/hadron-kernel/src/sync/`). Every
primitive is `const`-constructable so it can live in a `static` item and is
usable before the heap allocator or async executor is available.

The primitives fall into three categories:

| Category | Primitives | Requires heap | Requires executor |
|---|---|---|---|
| Spin-based | `SpinLock`, `IrqSpinLock`, `RwLock` | No | No |
| Async-aware | `Mutex`, `WaitQueue`, `HeapWaitQueue` | Partial | Yes |
| Initialization | `LazyLock` | No | No |

## SpinLock

**File:** `sync/spinlock.rs`
**Types:** `SpinLock<T>`, `SpinLockGuard<'a, T>`

A basic mutual-exclusion lock that busy-waits until the lock is available.
Uses test-and-test-and-set (TTAS) to reduce cache-line contention: the
acquire loop first attempts a `compare_exchange_weak` on the `AtomicBool`,
then falls back to spinning on a relaxed load (shared cache-line read) until
the lock appears free.

```rust
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}
```

### API

- `SpinLock::new(value)` -- const constructor.
- `lock() -> SpinLockGuard` -- spins until acquired; the guard implements
  `Deref` and `DerefMut`.
- `try_lock() -> Option<SpinLockGuard>` -- single non-blocking attempt;
  useful in panic handlers where spinning risks deadlock.
- `unsafe force_get() -> &mut T` -- bypasses the lock entirely. Last-resort
  escape hatch for uniprocessor panic paths.

The guard releases the lock on `Drop` with a `Release` store. `SpinLock<T>`
is `Send + Sync` when `T: Send`.

### When to use

Use `SpinLock` for short critical sections in non-interrupt code paths where
the protected data does not need to be accessed from interrupt handlers. It
is the lowest-overhead lock and the default choice for kernel data
structures during early boot.

## IrqSpinLock

**File:** `sync/irq_spinlock.rs`
**Types:** `IrqSpinLock<T>`, `IrqSpinLockGuard<'a, T>`

A spin lock that disables interrupts before acquiring the inner lock and
restores the previous interrupt state on release. This prevents a classic
deadlock scenario: if an interrupt handler tries to acquire a lock that the
interrupted code already holds, the CPU would spin forever with interrupts
disabled.

```rust
pub struct IrqSpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}
```

### Interrupt state management

On x86_64, the guard saves the `RFLAGS` register (via `pushfq; pop`)
before executing `cli`. On drop, it checks the saved interrupt flag
(bit 9) and only executes `sti` if interrupts were previously enabled.
This means nested `IrqSpinLock` acquisitions work correctly: the inner
lock's drop does not re-enable interrupts if the outer lock had already
disabled them.

An AArch64 implementation is also provided, using `DAIF` save/restore.
For host-side tests (`target_os != "none"`), the save/restore functions
are no-ops.

### Guard is `!Send`

`IrqSpinLockGuard` explicitly implements `!Send`. Interrupt state is
per-CPU, so migrating a guard to another CPU and restoring the wrong
CPU's flags would be unsound.

### When to use

Use `IrqSpinLock` whenever the protected data may be accessed from both
normal kernel code and interrupt handlers, or whenever you need to
guarantee that the critical section runs without being preempted by an
interrupt. Both `WaitQueue` and `HeapWaitQueue` use `IrqSpinLock`
internally for their waker storage.

## Mutex

**File:** `sync/mutex.rs`
**Types:** `Mutex<T>`, `MutexGuard<'a, T>`, `MutexLockFuture<'a, T>`

An async-aware mutual-exclusion lock. Unlike `SpinLock`, a contended
`Mutex` yields the current task back to the executor via a `WaitQueue`
rather than busy-waiting, allowing other async tasks to make progress.

```rust
pub struct Mutex<T> {
    locked: AtomicBool,
    waiters: WaitQueue,
    data: UnsafeCell<T>,
}
```

### API

- `Mutex::new(value)` -- const constructor (embeds a `WaitQueue`).
- `lock() -> MutexLockFuture` -- returns a future that resolves to
  `MutexGuard` once the lock is acquired.
- `try_lock() -> Option<MutexGuard>` -- single non-blocking attempt.
- `lock_sync() -> MutexGuard` -- synchronous spin-acquire; intended for
  initialization code or contexts where async is not available.

### Future polling strategy

`MutexLockFuture::poll` uses a careful protocol to avoid lost wakeups:

1. **Fast path:** attempt `compare_exchange_weak`. If it succeeds, return
   `Poll::Ready` immediately.
2. **Register waker:** call `WaitQueue::register_waker` with the task's
   `Waker`.
3. **Retry:** attempt acquisition again. The lock may have been released
   between step 1 and step 2, so retrying prevents a lost wakeup.
4. **Fallback:** if the `WaitQueue` is full (capacity 32), the future
   calls `cx.waker().wake_by_ref()` to self-wake, degrading gracefully
   to spin-poll behavior.

When the `MutexGuard` is dropped, it stores `false` (release) and calls
`WaitQueue::wake_one()` to resume the next waiting task.

### When to use

Use `Mutex` for any critical section in async task code where the lock
may be held across `.await` points or where contention is expected. It
integrates with the kernel's cooperative executor so that waiting tasks
yield CPU time rather than burning cycles.

## RwLock

**File:** `sync/rwlock.rs`
**Types:** `RwLock<T>`, `RwLockReadGuard<'a, T>`, `RwLockWriteGuard<'a, T>`

A spinning reader-writer lock that allows multiple concurrent readers or a
single exclusive writer.

```rust
pub struct RwLock<T> {
    state: AtomicU32,
    data: UnsafeCell<T>,
}
```

### State encoding

The lock state is packed into a single `AtomicU32`:

| Value | Meaning |
|---|---|
| `0` | Unlocked |
| `1..u32::MAX-1` | Read-locked with N active readers |
| `u32::MAX` | Write-locked |

Read acquisition increments the counter via `compare_exchange_weak`;
write acquisition transitions from `0` to `u32::MAX`. Both spin on
failure.

### API

- `RwLock::new(value)` -- const constructor.
- `read() -> RwLockReadGuard` -- spins until no writer holds the lock.
- `write() -> RwLockWriteGuard` -- spins until state is 0 (no readers,
  no writer).
- `try_read() -> Option<RwLockReadGuard>` -- non-blocking read attempt.
- `try_write() -> Option<RwLockWriteGuard>` -- non-blocking write attempt.

`RwLockReadGuard` implements `Deref` only (shared access).
`RwLockWriteGuard` implements both `Deref` and `DerefMut`.

### Sync bounds

`RwLock<T>` is `Sync` when `T: Send + Sync` (note the additional `Sync`
bound compared to `SpinLock`, since multiple readers access `T`
concurrently through shared references).

### When to use

Use `RwLock` for data that is read frequently but written rarely, such
as routing tables, configuration state, or device registries. The
read path allows full concurrency while the write path requires
exclusive access.

## WaitQueue

**File:** `sync/waitqueue.rs`
**Types:** `WaitQueue`, `WaitFuture<'a>`

A fixed-capacity queue of `Waker`s for interrupt-driven wakeups. Tasks
register their waker by calling `wait()` (which returns a future) or
`register_waker()`. Interrupt handlers or other kernel code call
`wake_one()` or `wake_all()` to resume waiting tasks.

```rust
pub struct WaitQueue {
    waiters: IrqSpinLock<ArrayVec<Waker, MAX_WAITERS>>,
}
```

The internal storage is a `noalloc::ArrayVec<Waker, 32>` protected by an
`IrqSpinLock`, so `WaitQueue` is usable before the heap allocator is
available and is safe to wake from interrupt context.

### API

- `WaitQueue::new()` -- const constructor.
- `wait() -> WaitFuture` -- returns a future that pends on the first poll
  (registering the waker) and completes on the second poll.
- `register_waker(&Waker) -> bool` -- manually register a waker; returns
  `false` if the queue is full (32 slots).
- `wake_one()` -- wakes the first registered waker (FIFO order via
  `swap_remove(0)`).
- `wake_all()` -- drains all wakers into a temporary buffer, drops the
  lock, then wakes each one outside the critical section.

### Capacity limit

The fixed capacity of 32 waiters is a deliberate trade-off: it keeps
`WaitQueue` allocation-free and suitable for frame-layer primitives (the
`Mutex` itself, low-level I/O completion). When more waiters are needed,
use `HeapWaitQueue` instead.

## HeapWaitQueue

**File:** `sync/heap_waitqueue.rs`
**Types:** `HeapWaitQueue`, `HeapWaitFuture<'a>`

A heap-backed wait queue with unbounded capacity, using
`alloc::collections::VecDeque<Waker>` for O(1) FIFO `wake_one()` via
`pop_front()`.

```rust
pub struct HeapWaitQueue {
    waiters: IrqSpinLock<VecDeque<Waker>>,
}
```

### API

The API mirrors `WaitQueue`:

- `HeapWaitQueue::new()` -- const constructor (empty `VecDeque`).
- `wait() -> HeapWaitFuture` -- future-based waiting.
- `register_waker(&Waker)` -- always succeeds (no capacity limit; note
  the return type is `()` rather than `bool`).
- `wake_one()` -- pops the front waker and wakes it.
- `wake_all()` -- drains the entire `VecDeque` via `mem::take`, drops
  the lock, then wakes all collected wakers.

### When to use

Use `HeapWaitQueue` for service-layer primitives (channels, barriers,
condition variables) where the number of concurrent waiters is
unpredictable. It requires the heap allocator to be initialized.

## LazyLock

**File:** `sync/lazy.rs`
**Types:** `LazyLock<T, F>`

A `no_std` equivalent of `std::sync::LazyLock` that initializes a value on
first access. Uses an atomic state machine with four states:

| State | Value | Meaning |
|---|---|---|
| `UNINIT` | 0 | Not yet initialized |
| `INITIALIZING` | 1 | One thread is running the init closure |
| `READY` | 2 | Value is available |
| `POISONED` | 3 | Init closure panicked |

```rust
pub struct LazyLock<T, F = fn() -> T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
    init: UnsafeCell<Option<F>>,
}
```

### Initialization protocol

1. A thread loads the state. If `READY`, it returns a reference immediately
   (fast path).
2. If `UNINIT`, it attempts a CAS to `INITIALIZING`. The winner creates an
   `InitGuard` (a drop-guard that transitions to `POISONED` on unwind),
   takes the init closure from the `Option`, calls it, writes the result
   into the `MaybeUninit`, stores `READY`, and defuses the guard.
3. Losers (and threads that observe `INITIALIZING`) spin until the state
   becomes `READY` or `POISONED`.

### Poisoning

If the init closure panics under `panic = unwind` (e.g., in host-side
tests), the `InitGuard`'s `Drop` impl stores `POISONED`. Subsequent
accesses panic with a descriptive message. Under the kernel's
`panic = abort` configuration, a panic halts the kernel immediately, so
the poisoned state is never observed in practice.

### API

`LazyLock` implements `Deref<Target = T>`, so accessing the value is
simply `*lazy` or `lazy.some_method()`. There is no explicit `get()` or
`init()` method -- dereferencing forces initialization.

### When to use

Use `LazyLock` for kernel globals that require non-trivial initialization
(allocating data structures, reading hardware registers) but must be
accessible as simple `static` references. Common examples include the
global allocator metadata, interrupt controller handles, and PCI device
tables.

## Usage Guidelines

### Decision matrix

| Scenario | Primitive |
|---|---|
| Short critical section, no interrupts involved | `SpinLock` |
| Data shared with interrupt handlers | `IrqSpinLock` |
| Critical section in async task code | `Mutex` |
| Read-heavy, write-rare data | `RwLock` |
| Async tasks waiting for an event (bounded waiters) | `WaitQueue` |
| Async tasks waiting for an event (unbounded waiters) | `HeapWaitQueue` |
| One-time lazy initialization of a `static` | `LazyLock` |

### General rules

1. **Prefer `Mutex` in async code.** The async `Mutex` yields to the
   executor when contended, keeping the cooperative scheduling model
   efficient. Use `SpinLock` or `IrqSpinLock` only when the critical
   section is too short to justify async overhead, or when the code runs
   outside the executor (early boot, interrupt handlers).

2. **Use `IrqSpinLock` whenever interrupts are involved.** If there is
   any chance that an interrupt handler touches the same data, use
   `IrqSpinLock`. Using a plain `SpinLock` in this scenario leads to
   deadlock: the interrupted thread holds the lock, the interrupt handler
   spins on it, and interrupts are never re-enabled.

3. **Keep critical sections short.** All spin-based primitives
   (`SpinLock`, `IrqSpinLock`, `RwLock`) busy-wait. Long critical
   sections waste CPU cycles and increase interrupt latency (especially
   with `IrqSpinLock`, which disables interrupts entirely).

4. **Do not hold spin locks across `.await` points.** A spin lock guard
   that lives across a yield point blocks other tasks from acquiring the
   lock while the holding task is suspended. Use `Mutex` instead.

5. **Mind the `WaitQueue` capacity.** The fixed-capacity `WaitQueue`
   (32 slots) is appropriate for frame-layer primitives where the number
   of concurrent waiters is bounded. For service-layer constructs
   (channels, barriers), use `HeapWaitQueue`.

6. **Use `LazyLock` instead of `SpinLock<Option<T>>` for init-once
   globals.** `LazyLock` expresses the "initialize exactly once" pattern
   more clearly and avoids the runtime cost of checking `Option::is_some`
   on every access after initialization.
