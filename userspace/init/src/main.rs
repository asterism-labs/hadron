//! Hadron init process — minimal launcher.
//!
//! Runs as PID 1 in userspace (ring 3). Sets default environment variables,
//! spawns the `/bin/sh` interactive shell, and respawns it if it exits.

#![no_std]
#![no_main]

use lepton_syslib::io::{self, STDIN, STDOUT};
use lepton_syslib::{env, println, sys};

/// Standard error fd.
const STDERR: usize = 2;

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    // Set default environment variables.
    env::setenv("PATH", "/bin");
    env::setenv("PWD", "/");
    env::setenv("HOME", "/");

    // Open /dev/tty0 and set up stdin/stdout/stderr.
    let tty_fd = io::open("/dev/tty0", 3); // READ | WRITE
    if tty_fd >= 0 {
        let tty_fd = tty_fd as usize;
        sys::dup2(tty_fd, STDIN);
        sys::dup2(tty_fd, STDOUT);
        sys::dup2(tty_fd, STDERR);
        if tty_fd > STDERR {
            io::close(tty_fd);
        }
    }

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
