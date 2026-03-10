//! Per-CPU span stack for scoped logging context.
//!
//! Spans are `&'static str` labels pushed onto a per-CPU stack. Each log
//! record captures a [`SpanSnapshot`] of the current stack, providing
//! hierarchical context without heap allocation.

use core::cell::UnsafeCell;
use core::marker::PhantomData;

use hadron_core::cpu_local::CpuLocal;

/// Maximum nesting depth for span labels.
pub const MAX_SPAN_DEPTH: usize = 8;

/// Per-CPU span stack. No allocation, no locks.
struct SpanStack {
    labels: [Option<&'static str>; MAX_SPAN_DEPTH],
    depth: u8,
}

impl SpanStack {
    const fn new() -> Self {
        Self {
            labels: [None; MAX_SPAN_DEPTH],
            depth: 0,
        }
    }

    fn push(&mut self, label: &'static str) {
        let d = self.depth as usize;
        if d < MAX_SPAN_DEPTH {
            self.labels[d] = Some(label);
            self.depth += 1;
        }
        // Silently drop if stack is full — no panics in logging.
    }

    fn pop(&mut self) {
        if self.depth > 0 {
            self.depth -= 1;
            self.labels[self.depth as usize] = None;
        }
    }

    fn snapshot(&self) -> SpanSnapshot {
        SpanSnapshot {
            labels: self.labels,
            depth: self.depth,
        }
    }
}

/// Snapshot of the span chain at log time, copied into each [`LogRecord`].
#[derive(Clone, Copy)]
pub struct SpanSnapshot {
    /// Span labels from outermost (index 0) to innermost.
    pub labels: [Option<&'static str>; MAX_SPAN_DEPTH],
    /// Number of active spans.
    pub depth: u8,
}

impl SpanSnapshot {
    /// Creates an empty span snapshot (no active spans).
    pub const fn empty() -> Self {
        Self {
            labels: [None; MAX_SPAN_DEPTH],
            depth: 0,
        }
    }

    /// Returns an iterator over the active span labels, outermost first.
    pub fn iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.labels[..self.depth as usize].iter().filter_map(|s| *s)
    }
}

/// RAII guard that pops the current span on drop.
///
/// `!Send` to prevent cross-CPU migration — a span must be entered and
/// exited on the same CPU.
pub struct SpanGuard {
    _not_send: PhantomData<*mut ()>,
}

impl Drop for SpanGuard {
    fn drop(&mut self) {
        if hadron_core::cpu_local::cpu_is_initialized() {
            // SAFETY: Per-CPU access, no concurrent mutation on the same CPU
            // (ISRs may nest but will push/pop their own spans correctly).
            let stack = SPAN_STACKS.get();
            unsafe { &mut *stack.get() }.pop();
        }
    }
}

// ── Per-CPU storage ─────────────────────────────────────────────────────

// SAFETY: Each CPU accesses only its own slot. ISRs on the same CPU see
// the interrupted code's span stack and may push/pop their own spans.
// This is safe because spans are strictly nested (RAII guards).
static SPAN_STACKS: CpuLocal<UnsafeCell<SpanStack>> = {
    // SAFETY: `UnsafeCell<SpanStack>` is init with const `SpanStack::new()`.
    // Each CPU slot is independent.
    const INIT: UnsafeCell<SpanStack> = UnsafeCell::new(SpanStack::new());
    CpuLocal::new([INIT; hadron_core::cpu_local::MAX_CPUS])
};

/// Pushes a span label onto the current CPU's span stack.
///
/// Returns a [`SpanGuard`] that pops the span on drop. The guard is `!Send`
/// to prevent cross-CPU migration.
pub fn enter_span(label: &'static str) -> SpanGuard {
    if hadron_core::cpu_local::cpu_is_initialized() {
        // SAFETY: Per-CPU exclusive access (see SPAN_STACKS safety comment).
        let stack = SPAN_STACKS.get();
        unsafe { &mut *stack.get() }.push(label);
    }
    SpanGuard {
        _not_send: PhantomData,
    }
}

/// Returns a snapshot of the current CPU's span stack.
pub fn current_spans() -> SpanSnapshot {
    if hadron_core::cpu_local::cpu_is_initialized() {
        // SAFETY: Per-CPU exclusive access.
        let stack = SPAN_STACKS.get();
        unsafe { &*stack.get() }.snapshot()
    } else {
        SpanSnapshot::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_stack_push_pop() {
        let mut stack = SpanStack::new();
        assert_eq!(stack.depth, 0);

        stack.push("outer");
        assert_eq!(stack.depth, 1);
        assert_eq!(stack.labels[0], Some("outer"));

        stack.push("inner");
        assert_eq!(stack.depth, 2);
        assert_eq!(stack.labels[1], Some("inner"));

        stack.pop();
        assert_eq!(stack.depth, 1);
        assert!(stack.labels[1].is_none());

        stack.pop();
        assert_eq!(stack.depth, 0);
    }

    #[test]
    fn span_stack_overflow_does_not_panic() {
        let mut stack = SpanStack::new();
        for _ in 0..MAX_SPAN_DEPTH + 4 {
            stack.push("x");
        }
        assert_eq!(stack.depth as usize, MAX_SPAN_DEPTH);
    }

    #[test]
    fn span_stack_underflow_does_not_panic() {
        let mut stack = SpanStack::new();
        stack.pop();
        assert_eq!(stack.depth, 0);
    }

    #[test]
    fn snapshot_iter() {
        let mut stack = SpanStack::new();
        stack.push("a");
        stack.push("b");
        let snap = stack.snapshot();
        let labels: Vec<_> = snap.iter().collect();
        assert_eq!(labels, &["a", "b"]);
    }

    #[test]
    fn empty_snapshot() {
        let snap = SpanSnapshot::empty();
        assert_eq!(snap.depth, 0);
        assert_eq!(snap.iter().count(), 0);
    }
}
