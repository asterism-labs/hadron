//! Hadron init process — minimal launcher.
//!
//! Runs as PID 1 in userspace (ring 3). Sets default environment variables,
//! spawns the `/bin/sh` interactive shell, and respawns it if it exits.

#![no_std]
#![no_main]

use lepton_syslib::{env, println, sys};

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    // Set default environment variables.
    env::setenv("PATH", "/bin");
    env::setenv("PWD", "/");
    env::setenv("HOME", "/");

    loop {
        let ret = sys::spawn("/bin/sh", &["/bin/sh"]);
        if ret < 0 {
            println!("init: failed to spawn /bin/sh (error {})", ret);
            return 1;
        }

        let child_pid = ret as u32;
        let mut status: u64 = 0;
        sys::waitpid(child_pid, Some(&mut status));

        println!("init: shell exited (status {}), respawning...", status);
    }
}
