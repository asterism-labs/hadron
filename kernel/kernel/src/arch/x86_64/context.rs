//! Kernel thread context switches.
//!
//! A switched-out thread's state is its stack. [`switch`] pushes the
//! callee-saved registers, stores RSP, loads the other thread's RSP and
//! pops its registers. The caller-saved registers are already saved by
//! the Rust code that called `switch`, as the System V ABI requires. The
//! kernel uses no floating-point or vector registers, so they need no
//! saving.

use core::arch::naked_asm;

/// Number of registers [`switch`] pushes.
const SAVED_REGISTERS: usize = 6;

/// Saves this thread's RSP to `*old` and resumes the thread whose RSP is
/// `new`. Returns when another switch resumes this thread.
///
/// # Safety
///
/// `old` must be writable, and `new` must be a stack pointer that
/// [`switch`] saved or that [`initial_stack`] built, for a stack that is
/// still mapped and that no running thread uses.
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn switch(old: *mut u64, new: u64) {
    naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",
        "mov rsp, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    );
}

/// Builds the frame [`switch`] pops for a new thread, below `top`, and
/// returns the stack pointer to switch to. The switch then returns into
/// `entry`.
///
/// # Safety
///
/// `top` must be the 16-byte aligned top of a mapped, writable stack with
/// room for the frame, and no one else may use the stack.
pub(crate) unsafe fn initial_stack(top: u64, entry: extern "C" fn() -> !) -> u64 {
    // From the top down: a zero return address for `entry`, which never
    // returns, then `entry` for `switch`'s `ret`, then the registers
    // `switch` pops, all zero. `entry` starts with RSP at 8 mod 16, as if
    // called, as the System V ABI requires.
    let words = SAVED_REGISTERS + 2;
    let rsp = top - (words * size_of::<u64>()) as u64;
    let frame = rsp as *mut u64;
    // SAFETY: the caller guarantees the stack is mapped and unused, and the
    // frame fits below `top`.
    unsafe {
        for i in 0..SAVED_REGISTERS {
            frame.add(i).write(0);
        }
        frame.add(SAVED_REGISTERS).write(entry as usize as u64);
        frame.add(SAVED_REGISTERS + 1).write(0);
    }
    rsp
}
