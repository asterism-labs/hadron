//! Spawn stress test.
//!
//! Spawns 8 copies of `/spawn-worker`, waits for all to exit, and reports
//! results. Tests cross-CPU scheduling, work stealing, and process lifecycle.

#![no_std]
#![no_main]

use lepton_syslib::{println, sys};

/// Number of workers to spawn.
const WORKER_COUNT: usize = 8;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let pid = sys::getpid();
    println!(
        "[spawn-stress PID {}] spawning {} workers...",
        pid, WORKER_COUNT
    );

    let mut child_pids = [0u32; WORKER_COUNT];
    let mut spawned = 0;

    for slot in &mut child_pids {
        let ret = sys::spawn("/spawn-worker");
        if ret < 0 {
            println!("[spawn-stress] failed to spawn worker (error {})", ret);
            continue;
        }
        *slot = ret as u32;
        spawned += 1;
    }

    println!(
        "[spawn-stress] spawned {}/{} workers, waiting...",
        spawned, WORKER_COUNT
    );

    let mut success = 0;
    let mut failed = 0;

    for &child_pid in &child_pids[..spawned] {
        let mut status: u64 = 0;
        let ret = sys::waitpid(child_pid, Some(&mut status));
        if ret < 0 {
            println!("[spawn-stress] waitpid({}) error {}", child_pid, ret);
            failed += 1;
        } else if status == 0 {
            success += 1;
        } else {
            println!(
                "[spawn-stress] worker {} exited with status {}",
                child_pid, status
            );
            failed += 1;
        }
    }

    println!(
        "[spawn-stress] done: {}/{} succeeded, {} failed",
        success, spawned, failed
    );

    if failed == 0 && success == spawned {
        0
    } else {
        1
    }
}
