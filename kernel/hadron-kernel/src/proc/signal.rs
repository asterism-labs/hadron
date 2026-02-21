//! Process signal infrastructure.
//!
//! Provides a bitmask-based pending signal set and signal-to-action mapping.
//! Each process has a [`SignalState`] that stores pending signals as bits in
//! an `AtomicU64`. Signal delivery is checked at kernel re-entry points
//! (after preemption, after blocking I/O, after waitpid).

use core::sync::atomic::{AtomicU64, Ordering};

use crate::syscall::{SIGCHLD, SIGINT, SIGKILL, SIGPIPE, SIGSEGV, SIGTERM};

/// Maximum signal number supported (bits 1..63).
const MAX_SIGNAL: usize = 63;

/// A Unix-style signal number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signal(pub usize);

impl Signal {
    /// Returns `true` if this signal number is valid (1..=`MAX_SIGNAL`).
    pub const fn is_valid(signum: usize) -> bool {
        signum >= 1 && signum <= MAX_SIGNAL
    }
}

/// Default action for a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalAction {
    /// Terminate the process.
    Terminate,
    /// Ignore the signal.
    Ignore,
}

/// Returns the default action for a signal number.
pub fn default_action(signum: usize) -> SignalAction {
    match signum {
        SIGINT | SIGKILL | SIGSEGV | SIGPIPE | SIGTERM => SignalAction::Terminate,
        SIGCHLD => SignalAction::Ignore,
        _ => SignalAction::Terminate, // Unknown signals terminate by default.
    }
}

/// Per-process signal state using an atomic bitmask.
///
/// Bit N represents signal number N (1-indexed, so bit 0 is unused).
/// This allows lock-free signal posting from interrupt context.
pub struct SignalState {
    /// Pending signal bitmask. Bit N = signal N is pending.
    pending: AtomicU64,
}

impl SignalState {
    /// Creates a new signal state with no pending signals.
    pub const fn new() -> Self {
        Self {
            pending: AtomicU64::new(0),
        }
    }

    /// Post a signal (set its pending bit).
    ///
    /// Can be called from any context (interrupt-safe).
    pub fn post(&self, signum: usize) {
        if Signal::is_valid(signum) {
            self.pending.fetch_or(1 << signum, Ordering::Release);
        }
    }

    /// Dequeue the highest-priority pending signal.
    ///
    /// Returns `Some(Signal)` and clears its pending bit, or `None` if
    /// no signals are pending. SIGKILL is always dequeued first.
    pub fn dequeue(&self) -> Option<Signal> {
        loop {
            let bits = self.pending.load(Ordering::Acquire);
            if bits == 0 {
                return None;
            }

            // SIGKILL (bit 9) has highest priority.
            let signum = if bits & (1 << SIGKILL) != 0 {
                SIGKILL
            } else {
                // Find lowest set bit (lowest signal number).
                bits.trailing_zeros() as usize
            };

            let mask = 1u64 << signum;
            // Atomically clear the bit. If another thread modified pending
            // between load and CAS, retry.
            match self.pending.compare_exchange_weak(
                bits,
                bits & !mask,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Signal(signum)),
                Err(_) => continue,
            }
        }
    }

    /// Returns `true` if any signal is pending.
    pub fn has_pending(&self) -> bool {
        self.pending.load(Ordering::Acquire) != 0
    }
}
