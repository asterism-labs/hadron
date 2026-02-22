//! Hadron init process — minimal multi-VT launcher.
//!
//! Runs as PID 1 in userspace (ring 3). Sets default environment variables,
//! spawns an interactive `/bin/sh` shell on each virtual terminal, and respawns
//! any shell that exits.

#![no_std]
#![no_main]

use lepton_syslib::io::{self, STDERR, STDIN, STDOUT};
use lepton_syslib::{env, println, sys};

/// Number of virtual terminals to spawn shells on.
const NUM_TTYS: usize = 6;

/// Device paths for each virtual terminal.
const TTY_PATHS: [&str; NUM_TTYS] = [
    "/dev/tty0",
    "/dev/tty1",
    "/dev/tty2",
    "/dev/tty3",
    "/dev/tty4",
    "/dev/tty5",
];

/// Spawn a shell on the given VT by opening its TTY device, pointing
/// stdin/stdout/stderr at it, and spawning `/bin/sh`.
///
/// After spawning, init's own fds are restored to `/dev/tty0`.
/// Returns the child PID on success.
fn spawn_shell_on_vt(vt: usize) -> Option<u32> {
    let tty_fd = io::open(TTY_PATHS[vt], 3); // READ | WRITE
    if tty_fd < 0 {
        return None;
    }
    let tty_fd = tty_fd as usize;

    // Point stdin/stdout/stderr at this VT's TTY.
    sys::dup2(tty_fd, STDIN);
    sys::dup2(tty_fd, STDOUT);
    sys::dup2(tty_fd, STDERR);
    if tty_fd > STDERR {
        io::close(tty_fd);
    }

    let ret = sys::spawn("/bin/sh", &["/bin/sh"]);

    // Restore init's own fds to tty0 so println! goes to the primary VT.
    restore_init_fds();

    if ret < 0 {
        return None;
    }
    Some(ret as u32)
}

/// Restore init's stdin/stdout/stderr to `/dev/tty0`.
fn restore_init_fds() {
    let fd = io::open(TTY_PATHS[0], 3); // READ | WRITE
    if fd >= 0 {
        let fd = fd as usize;
        sys::dup2(fd, STDIN);
        sys::dup2(fd, STDOUT);
        sys::dup2(fd, STDERR);
        if fd > STDERR {
            io::close(fd);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    // Set default environment variables.
    env::setenv("PATH", "/bin");
    env::setenv("PWD", "/");
    env::setenv("HOME", "/");

    // Track (child_pid, vt_index) for each spawned shell.
    let mut children: [(u32, usize); NUM_TTYS] = [(0, 0); NUM_TTYS];
    let mut count: usize = 0;

    // Spawn a shell on each VT.
    for vt in 0..NUM_TTYS {
        if let Some(pid) = spawn_shell_on_vt(vt) {
            children[count] = (pid, vt);
            count += 1;
        } else {
            println!("init: failed to spawn shell on tty{}", vt);
        }
    }

    if count == 0 {
        println!("init: no shells spawned, halting");
        return 1;
    }

    // Wait loop: reap any child, respawn its shell on the same VT.
    loop {
        let mut status: u64 = 0;
        let ret = sys::waitpid(0, Some(&mut status));
        if ret <= 0 {
            continue;
        }
        let exited_pid = ret as u32;

        // Find which VT this child was on and respawn.
        for entry in &mut children[..count] {
            if entry.0 == exited_pid {
                let vt = entry.1;
                println!("init: shell on tty{} exited (status {}), respawning...", vt, status);
                if let Some(new_pid) = spawn_shell_on_vt(vt) {
                    entry.0 = new_pid;
                } else {
                    println!("init: failed to respawn shell on tty{}", vt);
                }
                break;
            }
        }
    }
}
