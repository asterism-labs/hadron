//! Blocking sync-async bridge.
//!
//! Provides [`block_on`] for polling a future to completion by busy-waiting
//! with interrupt yields between polls. This is a temporary bridge for
//! synchronous code (hadris filesystem crates) that needs to call async
//! block device operations.

use core::future::Future;
use core::pin::pin;
use core::task::{Context, Poll};

/// Poll a future to completion, blocking the current CPU.
///
/// Between polls, enables interrupts and halts (`sti; hlt; cli`) to wait
/// for hardware events (disk IRQs, timers, etc.) without pure busy-spinning.
///
/// # Warning
///
/// This blocks the executor thread. Use only as a temporary bridge for
/// synchronous code calling async block device operations. Will be removed
/// when hadris crates support native async I/O.
pub fn block_on<T>(future: impl Future<Output = T>) -> T {
    let waker = crate::fs::noop_waker();
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => {
                // SAFETY: Standard interrupt-wait pattern. Enables interrupts,
                // halts until next IRQ, then disables interrupts before re-polling.
                unsafe {
                    core::arch::asm!("sti; hlt; cli", options(nomem, nostack));
                }
            }
        }
    }
}
