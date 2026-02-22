//! Process signal infrastructure.
//!
//! Provides a bitmask-based pending signal set, per-signal handler registration,
//! and signal-to-action mapping. Each process has a [`SignalState`] that stores
//! pending signals as bits in an `AtomicU64` and a handler table mapping signal
//! numbers to dispositions (`SIG_DFL`, `SIG_IGN`, or a userspace function pointer).
//! Signal delivery is checked at kernel re-entry points (after preemption, after
//! blocking I/O, after waitpid).

use core::sync::atomic::{AtomicU64, Ordering};

use crate::syscall::{SIGCHLD, SIGINT, SIGKILL, SIGPIPE, SIGQUIT, SIGSEGV, SIGSTOP, SIGTERM};
use crate::syscall::{SIG_DFL, SIG_IGN};

/// Maximum signal number supported (bits 1..63).
const MAX_SIGNAL: usize = 63;

/// Number of entries in the handler table (indexed 0..63, slot 0 unused).
const HANDLER_TABLE_SIZE: usize = 64;

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
        SIGINT | SIGKILL | SIGQUIT | SIGSEGV | SIGPIPE | SIGTERM => SignalAction::Terminate,
        SIGCHLD => SignalAction::Ignore,
        _ => SignalAction::Terminate, // Unknown signals terminate by default.
    }
}

/// Result of resolving how a signal should be handled.
#[derive(Debug, Clone, Copy)]
pub enum SignalDisposition {
    /// Apply the default action for this signal.
    Default(SignalAction),
    /// Ignore the signal entirely.
    Ignore,
    /// Deliver to a userspace handler at the given address.
    Handler(u64),
}

/// Per-process signal state using an atomic bitmask and handler table.
///
/// Bit N of `pending` represents signal number N (1-indexed, so bit 0 is unused).
/// `handlers[N]` stores the disposition for signal N: `SIG_DFL` (0), `SIG_IGN` (1),
/// or a userspace function pointer address.
///
/// Both pending and handlers are atomic for lock-free access from interrupt context.
pub struct SignalState {
    /// Pending signal bitmask. Bit N = signal N is pending.
    pending: AtomicU64,
    /// Per-signal handler table. `SIG_DFL` = 0, `SIG_IGN` = 1, else = handler addr.
    handlers: [AtomicU64; HANDLER_TABLE_SIZE],
}

impl SignalState {
    /// Creates a new signal state with no pending signals and all handlers set to `SIG_DFL`.
    pub const fn new() -> Self {
        Self {
            pending: AtomicU64::new(0),
            handlers: [const { AtomicU64::new(SIG_DFL as u64) }; HANDLER_TABLE_SIZE],
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

    /// Set the handler for a signal number. Returns the previous handler value.
    ///
    /// SIGKILL and SIGSTOP cannot be caught or ignored — returns `None` for those.
    pub fn set_handler(&self, signum: usize, handler: u64) -> Option<u64> {
        if !Signal::is_valid(signum) || signum == SIGKILL || signum == SIGSTOP {
            return None;
        }
        let old = self.handlers[signum].swap(handler, Ordering::AcqRel);
        Some(old)
    }

    /// Get the current handler for a signal number.
    pub fn get_handler(&self, signum: usize) -> u64 {
        if !Signal::is_valid(signum) {
            return SIG_DFL as u64;
        }
        self.handlers[signum].load(Ordering::Acquire)
    }

    /// Resolve how a signal should be handled based on the handler table.
    pub fn disposition(&self, signum: usize) -> SignalDisposition {
        // SIGKILL and SIGSTOP always use default action, regardless of handler table.
        if signum == SIGKILL || signum == SIGSTOP {
            return SignalDisposition::Default(default_action(signum));
        }

        let handler = self.get_handler(signum);
        match handler as usize {
            SIG_DFL => SignalDisposition::Default(default_action(signum)),
            SIG_IGN => SignalDisposition::Ignore,
            _ => SignalDisposition::Handler(handler),
        }
    }
}
