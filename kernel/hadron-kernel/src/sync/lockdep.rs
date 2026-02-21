//! Runtime lock dependency tracking (lockdep).
//!
//! Records "class A was held when class B was acquired" as a directed edge
//! in a dependency graph. On each new edge, runs DFS cycle detection. A cycle
//! indicates a potential deadlock even if it hasn't manifested yet.
//!
//! Gated behind `cfg(hadron_lockdep)`. Memory cost is ~17 KiB.

use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};

use crate::percpu::{CpuLocal, MAX_CPUS};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum distinct lock classes (static lock instances).
const MAX_CLASSES: usize = 64;

/// Maximum nesting depth per CPU (held-lock stack).
const MAX_HELD: usize = 16;

/// Maximum edges in the dependency graph.
const MAX_EDGES: usize = 256;

// ---------------------------------------------------------------------------
// Lock kind
// ---------------------------------------------------------------------------

/// The kind of lock (for diagnostic messages).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LockKind {
    /// A non-IRQ spinning lock.
    SpinLock,
    /// An interrupt-safe spinning lock.
    IrqSpinLock,
    /// An async-aware mutual exclusion lock.
    Mutex,
    /// A spinning reader-writer lock.
    RwLock,
}

impl LockKind {
    fn as_str(self) -> &'static str {
        match self {
            LockKind::SpinLock => "SpinLock",
            LockKind::IrqSpinLock => "IrqSpinLock",
            LockKind::Mutex => "Mutex",
            LockKind::RwLock => "RwLock",
        }
    }
}

// ---------------------------------------------------------------------------
// Lock class identification
// ---------------------------------------------------------------------------

/// Identifies a lock class. Classes are assigned by `get_or_register`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LockClassId(u16);

impl LockClassId {
    /// Sentinel value meaning "no class".
    pub const NONE: Self = Self(u16::MAX);

    fn index(self) -> usize {
        self.0 as usize
    }
}

/// Metadata for a registered lock class.
struct LockClassEntry {
    /// Lock address (for dedup on re-registration of the same static lock).
    addr: AtomicUsize,
    /// Human-readable name (e.g. `"PMM"`).
    name: &'static str,
    /// The kind of lock this class represents.
    kind: LockKind,
}

impl LockClassEntry {
    const fn empty() -> Self {
        Self {
            addr: AtomicUsize::new(0),
            name: "",
            kind: LockKind::SpinLock,
        }
    }
}

// ---------------------------------------------------------------------------
// Global state (lock-free)
// ---------------------------------------------------------------------------

/// Class table. Indexed by `LockClassId`.
static CLASSES: [LockClassEntry; MAX_CLASSES] = {
    const EMPTY: LockClassEntry = LockClassEntry::empty();
    [EMPTY; MAX_CLASSES]
};

/// Number of registered classes.
static CLASS_COUNT: AtomicU16 = AtomicU16::new(0);

/// Adjacency bitmap: `GRAPH[a * MAX_CLASSES + b]` is true when lock class `a`
/// was held while class `b` was acquired.
static GRAPH: [AtomicBool; MAX_CLASSES * MAX_CLASSES] = {
    const FALSE: AtomicBool = AtomicBool::new(false);
    [FALSE; MAX_CLASSES * MAX_CLASSES]
};

/// Edge metadata for diagnostics.
struct DepEdge {
    from: AtomicU16,
    to: AtomicU16,
}

impl DepEdge {
    const fn empty() -> Self {
        Self {
            from: AtomicU16::new(u16::MAX),
            to: AtomicU16::new(u16::MAX),
        }
    }
}

static EDGES: [DepEdge; MAX_EDGES] = {
    const EMPTY: DepEdge = DepEdge::empty();
    [EMPTY; MAX_EDGES]
};

static EDGE_COUNT: AtomicU32 = AtomicU32::new(0);

/// Raw spin protecting graph mutation (NOT a SpinLock — avoids self-tracking).
static GRAPH_LOCK: AtomicBool = AtomicBool::new(false);

fn graph_lock() {
    while GRAPH_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        while GRAPH_LOCK.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }
    }
}

fn graph_unlock() {
    GRAPH_LOCK.store(false, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Per-CPU held-lock stack
// ---------------------------------------------------------------------------

/// Stack of currently held lock classes on a single CPU.
struct HeldLocks {
    stack: [LockClassId; MAX_HELD],
    depth: usize,
}

impl HeldLocks {
    const fn new() -> Self {
        Self {
            stack: [LockClassId::NONE; MAX_HELD],
            depth: 0,
        }
    }

    fn push(&mut self, id: LockClassId) {
        if self.depth < MAX_HELD {
            self.stack[self.depth] = id;
            self.depth += 1;
        }
        // If we exceed MAX_HELD we silently stop tracking — better than
        // crashing inside the debug infrastructure.
    }

    fn pop(&mut self, id: LockClassId) {
        // Pop from top. In most cases the released lock is the most recently
        // acquired one (LIFO). If not (e.g. non-nested unlock), search down.
        for i in (0..self.depth).rev() {
            if self.stack[i] == id {
                // Shift everything above `i` down by one.
                let mut j = i;
                while j + 1 < self.depth {
                    self.stack[j] = self.stack[j + 1];
                    j += 1;
                }
                self.depth -= 1;
                return;
            }
        }
        // Not found — likely a mismatched release. Ignore silently.
    }
}

/// Per-CPU held-lock stacks.
static HELD: CpuLocal<core::cell::UnsafeCell<HeldLocks>> = CpuLocal::new(
    [const { core::cell::UnsafeCell::new(HeldLocks::new()) }; MAX_CPUS],
);

/// Per-CPU reentrancy guard. Prevents infinite recursion when lockdep
/// hooks trigger allocation or logging that re-enters lock acquisition.
static IN_LOCKDEP: CpuLocal<AtomicBool> =
    CpuLocal::new([const { AtomicBool::new(false) }; MAX_CPUS]);

// ---------------------------------------------------------------------------
// Early-boot guard
// ---------------------------------------------------------------------------

/// Returns `true` if lockdep hooks should be skipped (before per-CPU init).
#[inline]
fn should_skip() -> bool {
    #[cfg(target_os = "none")]
    {
        !crate::percpu::current_cpu().is_initialized()
    }
    #[cfg(not(target_os = "none"))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// Class registration
// ---------------------------------------------------------------------------

/// Registers a lock class by address. Returns the class ID. Idempotent.
pub fn get_or_register(addr: usize, name: &'static str, kind: LockKind) -> LockClassId {
    // Fast path: look for existing entry with same address.
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;
    for i in 0..count {
        if CLASSES[i].addr.load(Ordering::Relaxed) == addr {
            return LockClassId(i as u16);
        }
    }

    // Slow path: register new class under graph lock.
    graph_lock();

    // Re-check after acquiring lock (another CPU may have registered it).
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;
    for i in 0..count {
        if CLASSES[i].addr.load(Ordering::Relaxed) == addr {
            graph_unlock();
            return LockClassId(i as u16);
        }
    }

    if count >= MAX_CLASSES {
        graph_unlock();
        // Table full — return NONE so hooks become no-ops for this lock.
        return LockClassId::NONE;
    }

    // Write the entry. The `name` and `kind` fields are only written once
    // per slot, so we cast away the shared reference to write them.
    CLASSES[count].addr.store(addr, Ordering::Relaxed);
    // SAFETY: This slot is being initialized for the first time, and we hold
    // the graph lock so no concurrent writer exists. Readers only access
    // slots below CLASS_COUNT, which we haven't incremented yet.
    unsafe {
        let entry = &CLASSES[count] as *const LockClassEntry as *mut LockClassEntry;
        (*entry).name = name;
        (*entry).kind = kind;
    }

    // Publish the new class.
    CLASS_COUNT.store((count + 1) as u16, Ordering::Release);

    graph_unlock();
    LockClassId(count as u16)
}

// ---------------------------------------------------------------------------
// Lock acquire / release hooks
// ---------------------------------------------------------------------------

/// Called after a lock acquisition succeeds.
pub fn lock_acquired(class: LockClassId) {
    if class == LockClassId::NONE || should_skip() {
        return;
    }

    // Reentrancy guard.
    let in_lockdep = IN_LOCKDEP.get();
    if in_lockdep
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    // SAFETY: We are inside the reentrancy guard, so only this CPU touches
    // its own HeldLocks. Interrupts may fire but the reentrancy guard
    // prevents them from re-entering this code path.
    let held = unsafe { &mut *HELD.get().get() };

    // Record edges: for each currently held lock H, add edge H → class.
    for i in 0..held.depth {
        let h = held.stack[i];
        if h == LockClassId::NONE || h == class {
            continue;
        }

        let idx = h.index() * MAX_CLASSES + class.index();
        if !GRAPH[idx].load(Ordering::Relaxed) {
            // New edge — record it and check for cycles.
            graph_lock();
            if !GRAPH[idx].load(Ordering::Relaxed) {
                GRAPH[idx].store(true, Ordering::Relaxed);

                // Record edge metadata.
                let edge_idx = EDGE_COUNT.fetch_add(1, Ordering::Relaxed) as usize;
                if edge_idx < MAX_EDGES {
                    EDGES[edge_idx].from.store(h.0, Ordering::Relaxed);
                    EDGES[edge_idx].to.store(class.0, Ordering::Relaxed);
                }

                // Check for cycle: DFS from `class` looking for a path back to `h`.
                if has_path(class, h) {
                    graph_unlock();
                    report_cycle(h, class);
                    // Push the lock and bail — we've already reported.
                    held.push(class);
                    in_lockdep.store(false, Ordering::Release);
                    return;
                }
            }
            graph_unlock();
        }
    }

    held.push(class);
    in_lockdep.store(false, Ordering::Release);
}

/// Called before lock release (in guard `Drop`).
pub fn lock_released(class: LockClassId) {
    if class == LockClassId::NONE || should_skip() {
        return;
    }

    let in_lockdep = IN_LOCKDEP.get();
    if in_lockdep
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    // SAFETY: Same reasoning as lock_acquired — reentrancy guard ensures
    // single access to this CPU's HeldLocks.
    let held = unsafe { &mut *HELD.get().get() };
    held.pop(class);

    in_lockdep.store(false, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Cycle detection (DFS)
// ---------------------------------------------------------------------------

/// Returns `true` if there is a path from `src` to `dst` in the dependency graph.
///
/// Bounded by `MAX_CLASSES` (64 nodes). Only called when a new edge is
/// discovered, so after the graph stabilizes this never runs.
fn has_path(src: LockClassId, dst: LockClassId) -> bool {
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;
    let mut visited = [false; MAX_CLASSES];
    let mut stack = [0u16; MAX_CLASSES];
    let mut sp = 0;

    stack[sp] = src.0;
    sp += 1;

    while sp > 0 {
        sp -= 1;
        let node = stack[sp] as usize;

        if node == dst.index() {
            return true;
        }

        if visited[node] {
            continue;
        }
        visited[node] = true;

        // Explore outgoing edges from `node`.
        for neighbor in 0..count {
            let idx = node * MAX_CLASSES + neighbor;
            if GRAPH[idx].load(Ordering::Relaxed) && !visited[neighbor] {
                if sp < MAX_CLASSES {
                    stack[sp] = neighbor as u16;
                    sp += 1;
                }
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Violation reporting
// ---------------------------------------------------------------------------

/// Reports a lockdep cycle violation via direct serial output (no locks).
fn report_cycle(held: LockClassId, acquiring: LockClassId) {
    use core::fmt::Write;

    #[cfg(target_os = "none")]
    {
        use crate::drivers::early_console::{COM1, EarlySerial};
        use crate::log::SerialWriter;

        let mut w = SerialWriter(EarlySerial::new(COM1));

        let held_name = class_name(held);
        let held_kind = class_kind(held);
        let acq_name = class_name(acquiring);
        let acq_kind = class_kind(acquiring);

        let _ = write!(w, "\n!!! LOCKDEP: potential deadlock detected !!!\n");
        let _ = write!(w, "  Holding: \"{}\" ({})\n", held_name, held_kind.as_str());
        let _ = write!(w, "  Acquiring: \"{}\" ({})\n", acq_name, acq_kind.as_str());

        // Print the cycle path.
        let _ = write!(w, "  Cycle: \"{}\"", acq_name);
        print_path(&mut w, acquiring, held);
        let _ = write!(w, " -> \"{}\"\n", acq_name);
    }

    // In a debug build, panic after reporting.
    panic!(
        "lockdep: potential deadlock — held \"{}\" while acquiring \"{}\"",
        class_name(held),
        class_name(acquiring),
    );
}

/// Prints the shortest path from `src` to `dst` (BFS) for diagnostics.
#[cfg(target_os = "none")]
fn print_path(w: &mut impl core::fmt::Write, src: LockClassId, dst: LockClassId) {
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;
    let mut parent = [u16::MAX; MAX_CLASSES];
    let mut visited = [false; MAX_CLASSES];
    let mut queue = [0u16; MAX_CLASSES];
    let mut head = 0;
    let mut tail = 0;

    queue[tail] = src.0;
    tail += 1;
    visited[src.index()] = true;

    while head < tail {
        let node = queue[head] as usize;
        head += 1;

        if node == dst.index() {
            break;
        }

        for neighbor in 0..count {
            let idx = node * MAX_CLASSES + neighbor;
            if GRAPH[idx].load(Ordering::Relaxed) && !visited[neighbor] {
                visited[neighbor] = true;
                parent[neighbor] = node as u16;
                if tail < MAX_CLASSES {
                    queue[tail] = neighbor as u16;
                    tail += 1;
                }
            }
        }
    }

    // Reconstruct path from dst back to src.
    let mut path = [u16::MAX; MAX_CLASSES];
    let mut len = 0;
    let mut cur = dst.0;
    while cur != src.0 && cur != u16::MAX && len < MAX_CLASSES {
        path[len] = cur;
        len += 1;
        cur = parent[cur as usize];
    }

    // Print in forward order.
    for i in (0..len).rev() {
        let _ = write!(w, " -> \"{}\"", class_name(LockClassId(path[i])));
    }
}

/// Returns the name of a lock class.
fn class_name(id: LockClassId) -> &'static str {
    let idx = id.index();
    if idx < CLASS_COUNT.load(Ordering::Acquire) as usize {
        CLASSES[idx].name
    } else {
        "<unknown>"
    }
}

/// Returns the kind of a lock class.
fn class_kind(id: LockClassId) -> LockKind {
    let idx = id.index();
    if idx < CLASS_COUNT.load(Ordering::Acquire) as usize {
        CLASSES[idx].kind
    } else {
        LockKind::SpinLock
    }
}
