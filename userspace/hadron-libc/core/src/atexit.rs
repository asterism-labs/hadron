//! `atexit()` handler registration.
//!
//! POSIX function: `atexit`.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum number of atexit handlers.
const MAX_ATEXIT: usize = 32;

/// Registered handlers (function pointers stored as usize).
static HANDLERS: [AtomicUsize; MAX_ATEXIT] = {
    // const-initialize array of AtomicUsize(0)
    const ZERO: AtomicUsize = AtomicUsize::new(0);
    [ZERO; MAX_ATEXIT]
};

/// Number of registered handlers.
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// Register a function to be called at normal process termination.
///
/// Returns 0 on success, non-zero if the table is full.
#[unsafe(no_mangle)]
pub extern "C" fn atexit(func: extern "C" fn()) -> i32 {
    let idx = COUNT.fetch_add(1, Ordering::SeqCst);
    if idx >= MAX_ATEXIT {
        COUNT.fetch_sub(1, Ordering::SeqCst);
        return -1;
    }
    HANDLERS[idx].store(func as usize, Ordering::SeqCst);
    0
}

/// Run all registered handlers in LIFO order.
///
/// Called by `exit()` before flushing stdio and terminating.
pub fn run_handlers() {
    let n = COUNT.load(Ordering::SeqCst);
    // Run in reverse (LIFO) order.
    for i in (0..n.min(MAX_ATEXIT)).rev() {
        let fp = HANDLERS[i].load(Ordering::SeqCst);
        if fp != 0 {
            // SAFETY: The function pointer was registered via atexit().
            let f: extern "C" fn() = unsafe { core::mem::transmute(fp) };
            f();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::Ordering;

    // Reset state between tests (tests run serially per-process in cargo test).
    fn reset() {
        COUNT.store(0, Ordering::SeqCst);
        for h in &HANDLERS {
            h.store(0, Ordering::SeqCst);
        }
    }

    static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn inc_counter() {
        TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn test_atexit_register_and_run() {
        reset();
        TEST_COUNTER.store(0, Ordering::SeqCst);

        assert_eq!(atexit(inc_counter), 0);
        assert_eq!(atexit(inc_counter), 0);
        assert_eq!(COUNT.load(Ordering::SeqCst), 2);

        run_handlers();
        assert_eq!(TEST_COUNTER.load(Ordering::SeqCst), 2);
    }
}
