//! Process signal infrastructure.
//!
//! Provides a bitmask-based pending signal set, per-signal handler registration,
//! and signal-to-action mapping. Each process has a [`SignalState`] that stores
//! pending signals as bits in an `AtomicU64` and a handler table mapping signal
//! numbers to dispositions (`SIG_DFL`, `SIG_IGN`, or a userspace function pointer).
//! Signal delivery is checked at kernel re-entry points (after preemption, after
//! blocking I/O, after waitpid).

use hadron_core::sync::atomic::{AtomicU64, Ordering};

use crate::syscall::{SA_RESETHAND, SA_RESTART, SIG_DFL, SIG_IGN};
use crate::syscall::{SIGCHLD, SIGINT, SIGKILL, SIGPIPE, SIGQUIT, SIGSEGV, SIGSTOP, SIGTERM};

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

/// Compact flag bits used in the packed handler+flags representation.
/// External `SA_*` constants are converted to/from these on the API boundary.
const PACKED_FLAG_RESTART: u64 = 1 << 48;
const PACKED_FLAG_RESETHAND: u64 = 1 << 49;
const PACKED_HANDLER_MASK: u64 = (1 << 48) - 1;

/// Convert external `sa_flags` (u32, using `SA_RESTART`/`SA_RESETHAND` bit positions)
/// into the compact packed representation stored in the upper bits of handler entries.
fn pack_flags(sa_flags: u32) -> u64 {
    let mut packed = 0u64;
    if sa_flags & SA_RESTART as u32 != 0 {
        packed |= PACKED_FLAG_RESTART;
    }
    if sa_flags & SA_RESETHAND as u32 != 0 {
        packed |= PACKED_FLAG_RESETHAND;
    }
    packed
}

/// Convert packed flags back to external `sa_flags` format.
fn unpack_flags(packed: u64) -> u32 {
    let mut flags = 0u32;
    if packed & PACKED_FLAG_RESTART != 0 {
        flags |= SA_RESTART as u32;
    }
    if packed & PACKED_FLAG_RESETHAND != 0 {
        flags |= SA_RESETHAND as u32;
    }
    flags
}

/// Per-process signal state using an atomic bitmask and handler table.
///
/// Bit N of `pending` represents signal number N (1-indexed, so bit 0 is unused).
/// Each handler entry packs both the handler address (bits 0..48) and compact flags
/// (bits 48+) into a single `AtomicU64`, ensuring atomicity for `SA_RESETHAND`.
///
/// Both pending and handlers are atomic for lock-free access from interrupt context.
pub struct SignalState {
    /// Pending signal bitmask. Bit N = signal N is pending.
    pending: AtomicU64,
    /// Blocked signal bitmask. Bit N = signal N is blocked (masked).
    /// SIGKILL and SIGSTOP cannot be blocked.
    blocked: AtomicU64,
    /// Per-signal packed handler+flags table.
    /// Bits 0..48: handler address (`SIG_DFL`=0, `SIG_IGN`=1, else user fn pointer).
    /// Bits 48+: compact flag bits (`PACKED_FLAG_RESTART`, `PACKED_FLAG_RESETHAND`).
    handlers: [AtomicU64; HANDLER_TABLE_SIZE],
}

impl SignalState {
    /// Creates a new signal state with no pending signals, no mask, and all handlers `SIG_DFL`.
    pub const fn new() -> Self {
        Self {
            pending: AtomicU64::new(0),
            blocked: AtomicU64::new(0),
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

    /// Dequeue the highest-priority deliverable signal.
    ///
    /// Returns `Some(Signal)` and clears its pending bit, or `None` if
    /// no unblocked signals are pending. SIGKILL is always deliverable
    /// (cannot be blocked) and has highest priority.
    pub fn dequeue(&self) -> Option<Signal> {
        loop {
            let pending = self.pending.load(Ordering::Acquire);
            let blocked = self.blocked.load(Ordering::Acquire);
            // SIGKILL/SIGSTOP can never be blocked.
            let unblockable = (1u64 << SIGKILL) | (1u64 << SIGSTOP);
            let deliverable = pending & (!blocked | unblockable);

            if deliverable == 0 {
                return None;
            }

            // SIGKILL (bit 9) has highest priority.
            let signum = if deliverable & (1 << SIGKILL) != 0 {
                SIGKILL
            } else {
                // Find lowest set bit (lowest signal number).
                deliverable.trailing_zeros() as usize
            };

            let mask = 1u64 << signum;
            // Atomically clear the bit. If another thread modified pending
            // between load and CAS, retry.
            match self.pending.compare_exchange_weak(
                pending,
                pending & !mask,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Signal(signum)),
                Err(_) => continue,
            }
        }
    }

    /// Returns `true` if any deliverable (unblocked) signal is pending.
    pub fn has_pending(&self) -> bool {
        let pending = self.pending.load(Ordering::Acquire);
        let blocked = self.blocked.load(Ordering::Acquire);
        let unblockable = (1u64 << SIGKILL) | (1u64 << SIGSTOP);
        (pending & (!blocked | unblockable)) != 0
    }

    /// Get the current signal mask (blocked signals bitmask).
    pub fn get_mask(&self) -> u64 {
        self.blocked.load(Ordering::Acquire)
    }

    /// Set the signal mask. Returns the previous mask.
    ///
    /// `how` controls how `set` is applied:
    /// - `SIG_BLOCK` (0): block additional signals (`mask |= set`)
    /// - `SIG_UNBLOCK` (1): unblock signals (`mask &= !set`)
    /// - `SIG_SETMASK` (2): replace mask entirely
    ///
    /// SIGKILL and SIGSTOP bits are always cleared (cannot be blocked).
    pub fn set_mask(&self, how: usize, set: u64) -> u64 {
        let unblockable = (1u64 << SIGKILL) | (1u64 << SIGSTOP);
        loop {
            let old = self.blocked.load(Ordering::Acquire);
            let new = match how {
                0 => old | set,  // SIG_BLOCK
                1 => old & !set, // SIG_UNBLOCK
                2 => set,        // SIG_SETMASK
                _ => return old,
            } & !unblockable; // SIGKILL/SIGSTOP never blocked

            match self
                .blocked
                .compare_exchange_weak(old, new, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return old,
                Err(_) => continue,
            }
        }
    }

    /// Set the handler and flags for a signal number. Returns the previous handler value.
    ///
    /// SIGKILL and SIGSTOP cannot be caught or ignored — returns `None` for those.
    pub fn set_handler(&self, signum: usize, handler: u64, sa_flags: u32) -> Option<u64> {
        if !Signal::is_valid(signum) || signum == SIGKILL || signum == SIGSTOP {
            return None;
        }
        let packed = (handler & PACKED_HANDLER_MASK) | pack_flags(sa_flags);
        let old = self.handlers[signum].swap(packed, Ordering::AcqRel);
        Some(old & PACKED_HANDLER_MASK)
    }

    /// Get the current handler for a signal number.
    pub fn get_handler(&self, signum: usize) -> u64 {
        if !Signal::is_valid(signum) {
            return SIG_DFL as u64;
        }
        self.handlers[signum].load(Ordering::Acquire) & PACKED_HANDLER_MASK
    }

    /// Get the flags for a signal number.
    pub fn get_flags(&self, signum: usize) -> u32 {
        if !Signal::is_valid(signum) {
            return 0;
        }
        unpack_flags(self.handlers[signum].load(Ordering::Acquire))
    }

    /// Returns `true` if `SA_RESTART` is set for the given signal.
    pub fn has_restart(&self, signum: usize) -> bool {
        self.get_flags(signum) & SA_RESTART as u32 != 0
    }

    /// Reset all signal handlers to `SIG_DFL` (used by execve).
    pub fn reset_handlers(&self) {
        for i in 1..HANDLER_TABLE_SIZE {
            self.handlers[i].store(SIG_DFL as u64, Ordering::Release);
        }
    }

    /// Resolve how a signal should be handled based on the handler table.
    ///
    /// When `SA_RESETHAND` is set, the handler is atomically swapped to `SIG_DFL`
    /// using a CAS loop, preventing TOCTOU races with concurrent `set_handler` calls.
    pub fn disposition(&self, signum: usize) -> SignalDisposition {
        // SIGKILL and SIGSTOP always use default action, regardless of handler table.
        if signum == SIGKILL || signum == SIGSTOP {
            return SignalDisposition::Default(default_action(signum));
        }

        let packed = if Signal::is_valid(signum) {
            // Atomically load and, if SA_RESETHAND is set, swap to SIG_DFL.
            let mut current = self.handlers[signum].load(Ordering::Acquire);
            loop {
                let handler = current & PACKED_HANDLER_MASK;
                let needs_reset =
                    current & PACKED_FLAG_RESETHAND != 0 && handler as usize != SIG_DFL;
                if !needs_reset {
                    break current;
                }
                // CAS: atomically replace with SIG_DFL (no flags).
                match self.handlers[signum].compare_exchange_weak(
                    current,
                    SIG_DFL as u64,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break current,            // We won the race — use the old value.
                    Err(updated) => current = updated, // Retry with new value.
                }
            }
        } else {
            SIG_DFL as u64
        };

        let handler = packed & PACKED_HANDLER_MASK;
        match handler as usize {
            SIG_DFL => SignalDisposition::Default(default_action(signum)),
            SIG_IGN => SignalDisposition::Ignore,
            _ => SignalDisposition::Handler(handler),
        }
    }
}
