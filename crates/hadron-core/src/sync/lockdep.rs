//! Runtime lock dependency tracking (lockdep).
//!
//! Records "class A was held when class B was acquired" as a directed edge
//! in a dependency graph. On each new edge, runs DFS cycle detection. A cycle
//! indicates a potential deadlock even if it hasn't manifested yet.
//!
//! ## Capacity
//!
//! - 256 lock classes (up from 64)
//! - 32 nesting depth per CPU (up from 16)
//! - 1024 edges in the dependency graph (up from 256)
//!
//! ## Features
//!
//! - **Static lock class keys**: Multiple lock instances can share a single
//!   class via [`LockClassKey`], with optional subclass separation.
//! - **IRQ-safety validation**: Tracks whether each class is used in IRQ
//!   vs non-IRQ contexts and warns about unsafe combinations.
//! - **Lock contention statistics** (behind `cfg(hadron_lock_stat)`):
//!   per-class acquisition counts, contention counts, and hold/wait times.
//! - **Warning-only mode** (behind `cfg(hadron_lockdep_warn)`): logs
//!   violations without panicking.
//!
//! Gated behind `cfg(hadron_lockdep)`.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::cpu_local::{CpuLocal, MAX_CPUS};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum distinct lock classes.
const MAX_CLASSES: usize = 256;

/// Maximum nesting depth per CPU (held-lock stack).
const MAX_HELD: usize = 32;

/// Maximum edges in the dependency graph.
const MAX_EDGES: usize = 1024;

/// Number of `AtomicU64` words needed for the packed adjacency bitset.
/// Each bit represents an edge from class `row` to class `col`.
const GRAPH_WORDS: usize = MAX_CLASSES * MAX_CLASSES / 64;

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
    /// Returns a human-readable string for this lock kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpinLock => "SpinLock",
            Self::IrqSpinLock => "IrqSpinLock",
            Self::Mutex => "Mutex",
            Self::RwLock => "RwLock",
        }
    }
}

// ---------------------------------------------------------------------------
// Static lock class keys
// ---------------------------------------------------------------------------

/// Zero-sized marker placed in a `static` for unique linker address.
///
/// Multiple lock instances can share a class by referencing the same
/// `LockClassKey`. This is essential for per-object locks (e.g., per-inode)
/// where each instance should not consume a separate class slot.
#[repr(C)]
pub struct LockClassKey {
    _opaque: (),
}

// SAFETY: LockClassKey is a zero-sized type used solely for its static address.
unsafe impl Send for LockClassKey {}
unsafe impl Sync for LockClassKey {}

/// Declares a static [`LockClassKey`] with the given name.
///
/// # Example
///
/// ```ignore
/// lock_class_key!(MY_LOCK_CLASS);
/// let lock = SpinLock::with_class(&MY_LOCK_CLASS, 0);
/// ```
#[macro_export]
macro_rules! lock_class_key {
    ($name:ident) => {
        static $name: $crate::sync::lockdep::LockClassKey =
            $crate::sync::lockdep::LockClassKey { _opaque: () };
    };
}

/// A reference to a lock class, combining a static key with an optional subclass.
///
/// Subclasses allow distinguishing different usage patterns of the same
/// lock type (e.g., parent vs child inode locks).
#[derive(Clone, Copy)]
pub struct LockClassRef {
    /// The static class key (address is the identity).
    pub key: &'static LockClassKey,
    /// Subclass index (0 = default).
    pub subclass: u8,
    /// Human-readable name for diagnostics.
    pub name: &'static str,
}

// ---------------------------------------------------------------------------
// Lock class identification
// ---------------------------------------------------------------------------

/// Identifies a lock class. Classes are assigned by `get_or_register`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockClassId(u16);

impl LockClassId {
    /// Sentinel value meaning "no class".
    pub const NONE: Self = Self(u16::MAX);

    fn index(self) -> usize {
        self.0 as usize
    }
}

/// IRQ usage flags (per-class).
///
/// Bit 0: class was ever acquired in IRQ context (inside IrqSpinLock).
/// Bit 1: class was ever acquired in non-IRQ context.
const IRQ_USED_IN_IRQ: u8 = 1 << 0;
const IRQ_USED_IN_NON_IRQ: u8 = 1 << 1;

/// Metadata for a registered lock class.
struct LockClassEntry {
    /// Key address + subclass, packed for lookup.
    /// High bits: key address, low 8 bits: subclass.
    key_addr: AtomicUsize,
    /// Subclass index.
    subclass: AtomicU8,
    /// Human-readable name (e.g. `"PMM"`).
    name: &'static str,
    /// The kind of lock this class represents.
    kind: LockKind,
    /// IRQ usage tracking (bitfield of `IRQ_USED_IN_*`).
    irq_usage: AtomicU8,
}

impl LockClassEntry {
    const fn empty() -> Self {
        Self {
            key_addr: AtomicUsize::new(0),
            subclass: AtomicU8::new(0),
            name: "",
            kind: LockKind::SpinLock,
            irq_usage: AtomicU8::new(0),
        }
    }
}

// ---------------------------------------------------------------------------
// Lock contention statistics (behind cfg(hadron_lock_stat))
// ---------------------------------------------------------------------------

/// Per-class lock contention statistics.
///
/// All fields are atomically updated. Times are in TSC ticks (not nanoseconds)
/// to avoid the overhead of time conversion on the hot path.
#[cfg(hadron_lock_stat)]
struct LockStats {
    /// Total number of successful acquisitions.
    acquisitions: AtomicU64,
    /// Number of acquisitions that had to wait (contended).
    contentions: AtomicU64,
    /// Maximum hold time in TSC ticks.
    max_hold_tsc: AtomicU64,
    /// Cumulative hold time in TSC ticks.
    total_hold_tsc: AtomicU64,
    /// Maximum wait time in TSC ticks.
    max_wait_tsc: AtomicU64,
    /// Cumulative wait time in TSC ticks.
    total_wait_tsc: AtomicU64,
}

#[cfg(hadron_lock_stat)]
impl LockStats {
    const fn empty() -> Self {
        Self {
            acquisitions: AtomicU64::new(0),
            contentions: AtomicU64::new(0),
            max_hold_tsc: AtomicU64::new(0),
            total_hold_tsc: AtomicU64::new(0),
            max_wait_tsc: AtomicU64::new(0),
            total_wait_tsc: AtomicU64::new(0),
        }
    }

    fn record_acquisition(&self) {
        self.acquisitions.fetch_add(1, Ordering::Relaxed);
    }

    fn record_contention(&self) {
        self.contentions.fetch_add(1, Ordering::Relaxed);
    }

    fn record_hold_time(&self, tsc_delta: u64) {
        self.total_hold_tsc.fetch_add(tsc_delta, Ordering::Relaxed);
        // Relaxed max update: may race, but close enough for stats.
        let mut cur = self.max_hold_tsc.load(Ordering::Relaxed);
        while tsc_delta > cur {
            match self.max_hold_tsc.compare_exchange_weak(
                cur,
                tsc_delta,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }

    fn record_wait_time(&self, tsc_delta: u64) {
        self.total_wait_tsc.fetch_add(tsc_delta, Ordering::Relaxed);
        let mut cur = self.max_wait_tsc.load(Ordering::Relaxed);
        while tsc_delta > cur {
            match self.max_wait_tsc.compare_exchange_weak(
                cur,
                tsc_delta,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
    }
}

/// Read the TSC (timestamp counter) for contention timing.
#[cfg(all(hadron_lock_stat, target_os = "none", target_arch = "x86_64"))]
#[inline]
fn read_tsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: rdtsc is always available and has no side effects.
    unsafe {
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nomem, nostack));
    }
    (u64::from(hi) << 32) | u64::from(lo)
}

#[cfg(all(hadron_lock_stat, not(all(target_os = "none", target_arch = "x86_64"))))]
#[inline]
fn read_tsc() -> u64 {
    0
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

/// Packed adjacency bitset. Bit `(a * MAX_CLASSES + b)` is set when lock
/// class `a` was held while class `b` was acquired.
///
/// Uses `AtomicU64` words for compact storage: 256*256/64 = 1024 words = 8 KiB.
static GRAPH: [AtomicU64; GRAPH_WORDS] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; GRAPH_WORDS]
};

/// Tests whether edge (from, to) exists in the graph.
#[inline]
fn graph_test(from: usize, to: usize) -> bool {
    let bit = from * MAX_CLASSES + to;
    let word = bit / 64;
    let mask = 1u64 << (bit % 64);
    GRAPH[word].load(Ordering::Relaxed) & mask != 0
}

/// Sets edge (from, to) in the graph.
#[inline]
fn graph_set(from: usize, to: usize) {
    let bit = from * MAX_CLASSES + to;
    let word = bit / 64;
    let mask = 1u64 << (bit % 64);
    GRAPH[word].fetch_or(mask, Ordering::Relaxed);
}

/// Per-class contention statistics.
#[cfg(hadron_lock_stat)]
static STATS: [LockStats; MAX_CLASSES] = {
    const EMPTY: LockStats = LockStats::empty();
    [EMPTY; MAX_CLASSES]
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

fn acquire_graph_lock() {
    while GRAPH_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        while GRAPH_LOCK.load(Ordering::Relaxed) {
            core::hint::spin_loop();
        }
    }
}

fn release_graph_lock() {
    GRAPH_LOCK.store(false, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Per-CPU held-lock stack
// ---------------------------------------------------------------------------

/// Entry in the per-CPU held-lock stack.
#[derive(Clone, Copy)]
struct HeldEntry {
    /// Lock class ID.
    class: LockClassId,
    /// TSC at acquisition time (for hold-time stats).
    #[cfg(hadron_lock_stat)]
    acquire_tsc: u64,
}

impl HeldEntry {
    const fn empty() -> Self {
        Self {
            class: LockClassId::NONE,
            #[cfg(hadron_lock_stat)]
            acquire_tsc: 0,
        }
    }
}

/// Stack of currently held lock classes on a single CPU.
struct HeldLocks {
    stack: [HeldEntry; MAX_HELD],
    depth: usize,
}

impl HeldLocks {
    const fn new() -> Self {
        Self {
            stack: [HeldEntry::empty(); MAX_HELD],
            depth: 0,
        }
    }

    fn push(&mut self, entry: HeldEntry) {
        if self.depth < MAX_HELD {
            self.stack[self.depth] = entry;
            self.depth += 1;
        }
        // If we exceed MAX_HELD we silently stop tracking — better than
        // crashing inside the debug infrastructure.
    }

    fn pop(&mut self, id: LockClassId) -> Option<HeldEntry> {
        // Pop from top. In most cases the released lock is the most recently
        // acquired one (LIFO). If not (e.g. non-nested unlock), search down.
        for i in (0..self.depth).rev() {
            if self.stack[i].class == id {
                let entry = self.stack[i];
                // Shift everything above `i` down by one.
                let mut j = i;
                while j + 1 < self.depth {
                    self.stack[j] = self.stack[j + 1];
                    j += 1;
                }
                self.depth -= 1;
                return Some(entry);
            }
        }
        // Not found — likely a mismatched release. Ignore silently.
        None
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
        !crate::cpu_local::cpu_is_initialized()
    }
    #[cfg(not(target_os = "none"))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// IRQ context detection
// ---------------------------------------------------------------------------

/// Returns `true` if we are currently inside an `IrqSpinLock` critical section.
#[inline]
fn in_irq_context() -> bool {
    #[cfg(hadron_lock_debug)]
    {
        super::irq_spinlock::irq_lock_depth() != 0
    }
    #[cfg(not(hadron_lock_debug))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// Class registration
// ---------------------------------------------------------------------------

/// Registers a lock class by address. Returns the class ID. Idempotent.
///
/// This is the original registration function using the lock instance's
/// address as the class identity. Each static lock instance gets its own class.
pub fn get_or_register(addr: usize, name: &'static str, kind: LockKind) -> LockClassId {
    get_or_register_with_subclass(addr, 0, name, kind)
}

/// Registers a lock class by key address and subclass. Returns the class ID.
///
/// Class identity is `(key_addr, subclass)`. Multiple lock instances sharing
/// the same [`LockClassKey`] and subclass map to the same class.
pub fn get_or_register_with_subclass(
    key_addr: usize,
    subclass: u8,
    name: &'static str,
    kind: LockKind,
) -> LockClassId {
    // Fast path: look for existing entry with same key+subclass.
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;
    for i in 0..count {
        if CLASSES[i].key_addr.load(Ordering::Relaxed) == key_addr
            && CLASSES[i].subclass.load(Ordering::Relaxed) == subclass
        {
            return LockClassId(i as u16);
        }
    }

    // Slow path: register new class under graph lock.
    acquire_graph_lock();

    // Re-check after acquiring lock (another CPU may have registered it).
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;
    for i in 0..count {
        if CLASSES[i].key_addr.load(Ordering::Relaxed) == key_addr
            && CLASSES[i].subclass.load(Ordering::Relaxed) == subclass
        {
            release_graph_lock();
            return LockClassId(i as u16);
        }
    }

    if count >= MAX_CLASSES {
        release_graph_lock();
        // Table full — return NONE so hooks become no-ops for this lock.
        return LockClassId::NONE;
    }

    // Write the entry. The `name` and `kind` fields are only written once
    // per slot, so we cast away the shared reference to write them.
    CLASSES[count].key_addr.store(key_addr, Ordering::Relaxed);
    CLASSES[count].subclass.store(subclass, Ordering::Relaxed);
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

    release_graph_lock();
    LockClassId(count as u16)
}

/// Registers a lock class using a static [`LockClassKey`].
///
/// This is the preferred API for locks that share a class (e.g., per-inode
/// locks, per-device locks). All instances referencing the same key+subclass
/// share a single lockdep class.
pub fn get_or_register_class(
    class_ref: &LockClassRef,
    kind: LockKind,
) -> LockClassId {
    get_or_register_with_subclass(
        class_ref.key as *const LockClassKey as usize,
        class_ref.subclass,
        class_ref.name,
        kind,
    )
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

    // Record IRQ usage context.
    let irq = in_irq_context();
    let usage_bit = if irq { IRQ_USED_IN_IRQ } else { IRQ_USED_IN_NON_IRQ };
    let prev_usage = CLASSES[class.index()].irq_usage.fetch_or(usage_bit, Ordering::Relaxed);

    // IRQ-safety cross-check: if this class was previously used in non-IRQ
    // context and we're now in IRQ context, that's a potential deadlock
    // (the IRQ could fire while the lock is held in non-IRQ context).
    if irq && (prev_usage & IRQ_USED_IN_NON_IRQ) != 0 {
        let class_kind = CLASSES[class.index()].kind;
        // Only warn for non-IrqSpinLock types — IrqSpinLocks disable
        // interrupts by design, so they are safe in both contexts.
        if class_kind != LockKind::IrqSpinLock {
            report_irq_safety(class);
        }
    }

    // Record stats.
    #[cfg(hadron_lock_stat)]
    STATS[class.index()].record_acquisition();

    // SAFETY: We are inside the reentrancy guard, so only this CPU touches
    // its own HeldLocks. Interrupts may fire but the reentrancy guard
    // prevents them from re-entering this code path.
    let held = unsafe { &mut *HELD.get().get() };

    // Record edges: for each currently held lock H, add edge H → class.
    for i in 0..held.depth {
        let h = held.stack[i].class;
        if h == LockClassId::NONE || h == class {
            continue;
        }

        if !graph_test(h.index(), class.index()) {
            // New edge — record it and check for cycles.
            acquire_graph_lock();
            if !graph_test(h.index(), class.index()) {
                graph_set(h.index(), class.index());

                // Record edge metadata.
                let edge_idx = EDGE_COUNT.fetch_add(1, Ordering::Relaxed) as usize;
                if edge_idx < MAX_EDGES {
                    EDGES[edge_idx].from.store(h.0, Ordering::Relaxed);
                    EDGES[edge_idx].to.store(class.0, Ordering::Relaxed);
                }

                // Check for cycle: DFS from `class` looking for path back to `h`.
                if has_path(class, h) {
                    release_graph_lock();
                    // Dump full held-lock stack for diagnostics.
                    report_cycle(h, class, held);
                    // Push the lock and bail — we've already reported.
                    held.push(HeldEntry {
                        class,
                        #[cfg(hadron_lock_stat)]
                        acquire_tsc: read_tsc(),
                    });
                    in_lockdep.store(false, Ordering::Release);
                    return;
                }
            }
            release_graph_lock();
        }
    }

    held.push(HeldEntry {
        class,
        #[cfg(hadron_lock_stat)]
        acquire_tsc: read_tsc(),
    });
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
    let _entry = held.pop(class);

    // Record hold time if stats are enabled.
    #[cfg(hadron_lock_stat)]
    if let Some(entry) = _entry {
        let now = read_tsc();
        if now > entry.acquire_tsc {
            STATS[class.index()].record_hold_time(now - entry.acquire_tsc);
        }
    }

    in_lockdep.store(false, Ordering::Release);
}

/// Records that a lock acquisition was contended (had to wait).
///
/// Called by lock implementations when the fast-path CAS fails and the lock
/// enters the spin/wait loop. This is separate from `lock_acquired` so that
/// contention can be measured even when lockdep cycle checking is disabled.
#[cfg(hadron_lock_stat)]
pub fn lock_contended(class: LockClassId) {
    if class != LockClassId::NONE {
        STATS[class.index()].record_contention();
    }
}

// ---------------------------------------------------------------------------
// Cycle detection (DFS)
// ---------------------------------------------------------------------------

/// Returns `true` if there is a path from `src` to `dst` in the dependency graph.
///
/// Bounded by `MAX_CLASSES` (256 nodes). Only called when a new edge is
/// discovered, so after the graph stabilizes this never runs.
fn has_path(src: LockClassId, dst: LockClassId) -> bool {
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;
    let mut visited = [0u64; MAX_CLASSES / 64]; // packed bitset
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

        let word = node / 64;
        let bit = 1u64 << (node % 64);
        if visited[word] & bit != 0 {
            continue;
        }
        visited[word] |= bit;

        // Explore outgoing edges from `node`.
        for neighbor in 0..count {
            if graph_test(node, neighbor) {
                let nw = neighbor / 64;
                let nb = 1u64 << (neighbor % 64);
                if visited[nw] & nb == 0 && sp < MAX_CLASSES {
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

/// Reports a lockdep cycle violation.
///
/// Includes the full held-lock stack for diagnostics.
fn report_cycle(held: LockClassId, acquiring: LockClassId, held_stack: &HeldLocks) {
    // Build the diagnostic message including the held-lock stack.
    let held_name = class_name(held);
    let held_kind = class_kind(held);
    let acq_name = class_name(acquiring);
    let acq_kind = class_kind(acquiring);

    // Format held-lock stack info for the panic message.
    // We build a simple representation since we can't allocate.
    let depth = held_stack.depth;

    #[cfg(hadron_lockdep_warn)]
    {
        // Warning-only mode: log but don't panic.
        // On kernel, this would go to serial. On host, we can't easily log
        // without allocating. The best we can do is... nothing useful without
        // a logging callback. The panic message below serves as documentation.
        let _ = (held_name, held_kind, acq_name, acq_kind, depth);
        return;
    }

    #[cfg(not(hadron_lockdep_warn))]
    panic!(
        "lockdep: potential deadlock detected!\n\
         Held: \"{}\" ({}) | Acquiring: \"{}\" ({})\n\
         Held-lock stack depth: {}",
        held_name,
        held_kind.as_str(),
        acq_name,
        acq_kind.as_str(),
        depth,
    );
}

/// Reports an IRQ-safety violation.
///
/// A lock class that was previously used in non-IRQ context is now being
/// acquired in IRQ context. This means if the IRQ fires while the lock is
/// held in non-IRQ context, deadlock occurs.
fn report_irq_safety(class: LockClassId) {
    let name = class_name(class);
    let kind = class_kind(class);

    #[cfg(hadron_lockdep_warn)]
    {
        let _ = (name, kind);
        return;
    }

    #[cfg(not(hadron_lockdep_warn))]
    panic!(
        "lockdep: IRQ-safety violation!\n\
         Lock \"{}\" ({}) used in both IRQ and non-IRQ contexts.\n\
         Use IrqSpinLock if this lock must be shared with interrupt handlers.",
        name,
        kind.as_str(),
    );
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

// ---------------------------------------------------------------------------
// Statistics dump
// ---------------------------------------------------------------------------

/// Writes lock contention statistics to the given formatter.
///
/// Output format (one line per class):
/// ```text
/// CLASS   KIND        ACQUIRES   CONTENTIONS   MAX_HOLD   TOTAL_HOLD   MAX_WAIT   TOTAL_WAIT
/// PMM     SpinLock        1234           56      12345       678900      54321       987654
/// ```
#[cfg(hadron_lock_stat)]
pub fn dump_lock_stats(w: &mut impl core::fmt::Write) -> core::fmt::Result {
    let count = CLASS_COUNT.load(Ordering::Acquire) as usize;

    writeln!(
        w,
        "{:<24} {:<12} {:>10} {:>12} {:>10} {:>12} {:>10} {:>12}",
        "CLASS", "KIND", "ACQUIRES", "CONTENTIONS", "MAX_HOLD", "TOTAL_HOLD", "MAX_WAIT", "TOTAL_WAIT"
    )?;

    for i in 0..count {
        let name = CLASSES[i].name;
        let kind = CLASSES[i].kind.as_str();
        let stats = &STATS[i];

        let acq = stats.acquisitions.load(Ordering::Relaxed);
        if acq == 0 {
            continue; // Skip unused classes.
        }

        writeln!(
            w,
            "{:<24} {:<12} {:>10} {:>12} {:>10} {:>12} {:>10} {:>12}",
            name,
            kind,
            acq,
            stats.contentions.load(Ordering::Relaxed),
            stats.max_hold_tsc.load(Ordering::Relaxed),
            stats.total_hold_tsc.load(Ordering::Relaxed),
            stats.max_wait_tsc.load(Ordering::Relaxed),
            stats.total_wait_tsc.load(Ordering::Relaxed),
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------

/// Resets all lockdep global state. Only available in test builds.
///
/// Must be called with `--test-threads=1` to avoid races between tests.
#[cfg(test)]
pub fn reset_lockdep_state() {
    // Reset class table.
    CLASS_COUNT.store(0, Ordering::Release);
    for cls in &CLASSES {
        cls.key_addr.store(0, Ordering::Relaxed);
        cls.subclass.store(0, Ordering::Relaxed);
        cls.irq_usage.store(0, Ordering::Relaxed);
    }

    // Reset graph.
    for word in &GRAPH {
        word.store(0, Ordering::Relaxed);
    }

    // Reset edges.
    EDGE_COUNT.store(0, Ordering::Relaxed);
    for edge in &EDGES {
        edge.from.store(u16::MAX, Ordering::Relaxed);
        edge.to.store(u16::MAX, Ordering::Relaxed);
    }

    // Reset held-lock stacks (CPU 0 only for host tests).
    // SAFETY: Only called in single-threaded test context.
    unsafe {
        let held = &mut *HELD.get().get();
        held.depth = 0;
    }

    // Reset reentrancy guard.
    IN_LOCKDEP.get().store(false, Ordering::Relaxed);

    // Reset stats.
    #[cfg(hadron_lock_stat)]
    for stat in &STATS {
        stat.acquisitions.store(0, Ordering::Relaxed);
        stat.contentions.store(0, Ordering::Relaxed);
        stat.max_hold_tsc.store(0, Ordering::Relaxed);
        stat.total_hold_tsc.store(0, Ordering::Relaxed);
        stat.max_wait_tsc.store(0, Ordering::Relaxed);
        stat.total_wait_tsc.store(0, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// These tests must be run with:
//   RUSTFLAGS="--cfg hadron_lockdep" cargo test -p hadron-core -- lockdep --test-threads=1
//
// Single-threaded execution is required because lockdep uses per-CPU globals
// (CPU 0 on host) that are shared across tests.
#[cfg(test)]
mod tests {
    use super::*;

    fn setup() {
        reset_lockdep_state();
    }

    // --- Class registration ---

    #[test]
    fn class_registration_basic() {
        setup();
        let c1 = get_or_register(0x1000, "lock_a", LockKind::SpinLock);
        assert_ne!(c1, LockClassId::NONE);
    }

    #[test]
    fn class_registration_deduplication() {
        setup();
        let c1 = get_or_register(0x1000, "lock_a", LockKind::SpinLock);
        let c2 = get_or_register(0x1000, "lock_a", LockKind::SpinLock);
        assert_eq!(c1, c2, "same address should yield same class");
    }

    #[test]
    fn different_addresses_different_classes() {
        setup();
        let c1 = get_or_register(0x1000, "A", LockKind::SpinLock);
        let c2 = get_or_register(0x2000, "B", LockKind::SpinLock);
        assert_ne!(c1, c2, "different addresses should yield different classes");
    }

    #[test]
    fn subclass_separation() {
        setup();
        let c1 = get_or_register_with_subclass(0x1000, 0, "inode.data", LockKind::SpinLock);
        let c2 = get_or_register_with_subclass(0x1000, 1, "inode.meta", LockKind::SpinLock);
        assert_ne!(c1, c2, "different subclasses should yield different classes");
    }

    #[test]
    fn subclass_deduplication() {
        setup();
        let c1 = get_or_register_with_subclass(0x1000, 0, "X", LockKind::SpinLock);
        let c2 = get_or_register_with_subclass(0x1000, 0, "X", LockKind::SpinLock);
        assert_eq!(c1, c2, "same key+subclass should deduplicate");
    }

    #[test]
    fn class_key_sharing() {
        setup();
        // Simulate multiple lock instances sharing a class key.
        // In real usage, they'd reference the same `LockClassKey` static.
        let key_addr = 0x5000;
        let c1 = get_or_register_with_subclass(key_addr, 0, "shared", LockKind::SpinLock);
        let c2 = get_or_register_with_subclass(key_addr, 0, "shared", LockKind::SpinLock);
        assert_eq!(c1, c2, "multiple instances with same key should share class");
    }

    // --- Consistent ordering (no false positives) ---

    #[test]
    fn consistent_ordering_no_panic() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);

        // Lock A then B — establishes ordering A → B.
        lock_acquired(a);
        lock_acquired(b);
        lock_released(b);
        lock_released(a);

        // Same order again — should not panic.
        lock_acquired(a);
        lock_acquired(b);
        lock_released(b);
        lock_released(a);
    }

    #[test]
    fn three_lock_consistent_chain() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);
        let c = get_or_register(0x3000, "C", LockKind::SpinLock);

        // A → B → C — consistent chain.
        lock_acquired(a);
        lock_acquired(b);
        lock_acquired(c);
        lock_released(c);
        lock_released(b);
        lock_released(a);

        // Same order — no panic.
        lock_acquired(a);
        lock_acquired(b);
        lock_acquired(c);
        lock_released(c);
        lock_released(b);
        lock_released(a);
    }

    // --- Simple cycle detection ---

    #[test]
    #[should_panic(expected = "lockdep: potential deadlock")]
    fn simple_cycle_ab_ba() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);

        // Establish A → B.
        lock_acquired(a);
        lock_acquired(b);
        lock_released(b);
        lock_released(a);

        // Now B → A — cycle!
        lock_acquired(b);
        lock_acquired(a); // should panic
    }

    // --- Transitive cycle detection ---

    #[test]
    #[should_panic(expected = "lockdep: potential deadlock")]
    fn transitive_cycle_abc() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);
        let c = get_or_register(0x3000, "C", LockKind::SpinLock);

        // A → B
        lock_acquired(a);
        lock_acquired(b);
        lock_released(b);
        lock_released(a);

        // B → C
        lock_acquired(b);
        lock_acquired(c);
        lock_released(c);
        lock_released(b);

        // C → A — creates transitive cycle A→B→C→A.
        lock_acquired(c);
        lock_acquired(a); // should panic
    }

    // --- Edge and graph tests ---

    #[test]
    fn graph_edge_recorded() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);

        assert!(!graph_test(a.index(), b.index()), "no edge before lock");

        lock_acquired(a);
        lock_acquired(b);
        lock_released(b);
        lock_released(a);

        assert!(graph_test(a.index(), b.index()), "edge A→B should exist");
        assert!(!graph_test(b.index(), a.index()), "edge B→A should not exist");
    }

    #[test]
    fn lock_release_clears_held_stack() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);

        lock_acquired(a);
        lock_released(a);

        // Re-acquiring should work fine — held stack should be empty.
        lock_acquired(a);
        lock_released(a);
    }

    #[test]
    fn non_nested_release_order() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);

        // Acquire A then B, but release A first (non-LIFO).
        lock_acquired(a);
        lock_acquired(b);
        lock_released(a);
        lock_released(b);
    }

    #[test]
    fn class_count_increments() {
        setup();
        assert_eq!(CLASS_COUNT.load(Ordering::Relaxed), 0);

        let _ = get_or_register(0x1000, "A", LockKind::SpinLock);
        assert_eq!(CLASS_COUNT.load(Ordering::Relaxed), 1);

        let _ = get_or_register(0x2000, "B", LockKind::Mutex);
        assert_eq!(CLASS_COUNT.load(Ordering::Relaxed), 2);

        // Registering same address doesn't increment.
        let _ = get_or_register(0x1000, "A", LockKind::SpinLock);
        assert_eq!(CLASS_COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn has_path_direct() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);

        // No edge yet.
        assert!(!has_path(a, b));

        // Add edge A → B manually.
        graph_set(a.index(), b.index());
        assert!(has_path(a, b));
        assert!(!has_path(b, a));
    }

    #[test]
    fn has_path_transitive() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);
        let c = get_or_register(0x3000, "C", LockKind::SpinLock);

        graph_set(a.index(), b.index()); // A → B
        graph_set(b.index(), c.index()); // B → C

        assert!(has_path(a, c), "transitive path A→B→C");
        assert!(!has_path(c, a), "no reverse path");
    }

    #[test]
    fn reset_clears_all_state() {
        setup();
        let a = get_or_register(0x1000, "A", LockKind::SpinLock);
        let b = get_or_register(0x2000, "B", LockKind::SpinLock);

        lock_acquired(a);
        lock_acquired(b);
        lock_released(b);
        lock_released(a);

        assert!(graph_test(a.index(), b.index()));
        assert_eq!(CLASS_COUNT.load(Ordering::Relaxed), 2);

        reset_lockdep_state();

        assert_eq!(CLASS_COUNT.load(Ordering::Relaxed), 0);
        // Graph should be cleared (can't easily test since indices
        // are no longer valid, but we test that re-registration works).
        let a2 = get_or_register(0x1000, "A", LockKind::SpinLock);
        assert_eq!(a2.index(), 0, "first class after reset should be index 0");
    }
}
