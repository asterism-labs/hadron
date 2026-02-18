//! CPU-bound preemption demo.
//!
//! Infinite loop that busy-spins and periodically prints its PID and
//! iteration count. Demonstrates preemptive multitasking when multiple
//! instances run on different CPUs.

#![no_std]
#![no_main]

use hadron_syslib::{println, sys};

/// Iterations per print.
const SPIN_COUNT: u64 = 1_000_000;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let pid = sys::getpid();
    let mut iteration: u64 = 0;

    loop {
        for _ in 0..SPIN_COUNT {
            core::hint::black_box(0u64);
        }
        iteration += 1;
        println!("[spinner PID {}] iteration {}", pid, iteration);
    }
}
