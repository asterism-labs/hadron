//! Hadron init process — minimal launcher.
//!
//! Runs as PID 1 in userspace (ring 3). Spawns the `/shell` interactive
//! shell and respawns it if it exits. If spawning fails, prints an error
//! and exits.

#![no_std]
#![no_main]

use lepton_syslib::{println, sys};

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    loop {
        let ret = sys::spawn("/shell", &["/shell"]);
        if ret < 0 {
            println!("init: failed to spawn /shell (error {})", ret);
            return 1;
        }

        let child_pid = ret as u32;
        let mut status: u64 = 0;
        sys::waitpid(child_pid, Some(&mut status));

        println!("init: shell exited (status {}), respawning...", status);
    }
}
