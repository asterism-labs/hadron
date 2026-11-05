//! Interrupt vectors 32 to 255: which handler each one runs.
//!
//! Handlers run with interrupts masked, on the stack of the thread they
//! interrupted. They must not sleep. After the handler, [`dispatch`]
//! signals the end of the interrupt to the local APIC and lets the
//! scheduler switch threads.

use core::sync::atomic::{AtomicPtr, Ordering};

use super::lapic;
use crate::sched;

/// The first vector after the exceptions.
pub(crate) const FIRST: u8 = 32;

/// Number of interrupt vectors.
const COUNT: usize = 256 - FIRST as usize;

/// A handler for one vector.
pub(crate) type Handler = fn();

/// The handler of each vector, or null for none.
static HANDLERS: [AtomicPtr<()>; COUNT] = [const { AtomicPtr::new(core::ptr::null_mut()) }; COUNT];

/// Runs `handler` for every interrupt on `vector`.
///
/// # Panics
///
/// Panics if `vector` is an exception vector or already has a handler.
pub(crate) fn set_handler(vector: u8, handler: Handler) {
    assert!(vector >= FIRST, "vector {vector} is an exception vector");
    let slot = &HANDLERS[usize::from(vector - FIRST)];
    let set = slot
        .compare_exchange(
            core::ptr::null_mut(),
            handler as *mut (),
            Ordering::Release,
            Ordering::Relaxed,
        )
        .is_ok();
    assert!(set, "vector {vector} has a handler");
}

/// Called by the trap entry for vectors 32 to 255.
///
/// # Panics
///
/// Panics if `vector` has no handler.
pub(super) fn dispatch(vector: u8) {
    let raw = HANDLERS[usize::from(vector - FIRST)].load(Ordering::Acquire);
    assert!(
        !raw.is_null(),
        "interrupt on vector {vector}, which has no handler"
    );
    // SAFETY: `set_handler` stores only `Handler`s.
    let handler = unsafe { core::mem::transmute::<*mut (), Handler>(raw) };
    handler();
    // The APIC sets no in-service bit for a spurious interrupt, so it
    // takes no EOI. The EOI comes before a switch, or the APIC would hold
    // back interrupts until this thread runs again.
    if vector != lapic::SPURIOUS {
        lapic::eoi();
    }
    sched::preempt_on_interrupt_exit();
}

ktest::ktest! {
    fn software_interrupts_reach_their_handlers() {
        use core::sync::atomic::AtomicU32;

        static LOW: AtomicU32 = AtomicU32::new(0);
        static HIGH: AtomicU32 = AtomicU32::new(0);
        set_handler(32, || {
            LOW.fetch_add(1, Ordering::Relaxed);
        });
        set_handler(0xfe, || {
            HIGH.fetch_add(1, Ordering::Relaxed);
        });
        // SAFETY: both vectors have handlers that only count.
        unsafe {
            core::arch::asm!("int 32", "int 0xfe", "int 0xfe");
        }
        assert_eq!(LOW.load(Ordering::Relaxed), 1);
        assert_eq!(HIGH.load(Ordering::Relaxed), 2);
    }
}
