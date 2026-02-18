//! Short-lived worker child for spawn stress testing.
//!
//! Does light work (busy-spin ~100K iterations), queries uptime, prints
//! PID + uptime, and exits. Used as a child process by `hadron-spawn-stress`.

#![no_std]
#![no_main]

use hadron_syslib::{println, sys};

/// Light work iterations.
const WORK_COUNT: u64 = 100_000;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let pid = sys::getpid();

    for _ in 0..WORK_COUNT {
        core::hint::black_box(0u64);
    }

    if let Some(uptime) = sys::query_uptime() {
        let secs = uptime.uptime_ns / 1_000_000_000;
        let ms = (uptime.uptime_ns % 1_000_000_000) / 1_000_000;
        println!("[worker PID {}] done, uptime {}.{:03}s", pid, secs, ms);
    } else {
        println!("[worker PID {}] done", pid);
    }

    0
}
