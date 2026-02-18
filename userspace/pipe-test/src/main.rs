//! Pipe test orchestrator.
//!
//! Creates a pipe, redirects the read end to stdin via dup2, spawns
//! `/pipe-consumer` (which inherits the redirected stdin), writes test
//! messages through the pipe, closes the write end (signaling EOF),
//! and waits for the consumer to finish.

#![no_std]
#![no_main]

use hadron_syslib::io::{self, STDIN};
use hadron_syslib::{println, sys};

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let pid = sys::getpid();
    println!("[pipe-test PID {}] starting pipe test...", pid);

    // 1. Create pipe -> (read_fd, write_fd).
    let (read_fd, write_fd) = match sys::pipe() {
        Ok(fds) => fds,
        Err(e) => {
            println!("[pipe-test] pipe() failed: {}", e);
            return 1;
        }
    };
    println!(
        "[pipe-test] pipe created: read_fd={}, write_fd={}",
        read_fd, write_fd
    );

    // 2. Save our real stdin, then redirect stdin to pipe read end.
    //    dup2(read_fd, 0) — child will inherit this redirected stdin.
    let saved_stdin_fd = io::open("/dev/console", 1); // open READ
    if saved_stdin_fd < 0 {
        println!("[pipe-test] failed to open /dev/console for stdin save");
        return 1;
    }
    let saved_stdin_fd = saved_stdin_fd as usize;

    sys::dup2(read_fd, STDIN);

    // 3. Spawn /pipe-consumer — child inherits our redirected stdin.
    let ret = sys::spawn("/pipe-consumer");
    if ret < 0 {
        println!("[pipe-test] failed to spawn /pipe-consumer: {}", ret);
        return 1;
    }
    let child_pid = ret as u32;
    println!("[pipe-test] spawned pipe-consumer PID {}", child_pid);

    // 4. Restore our own stdin.
    sys::dup2(saved_stdin_fd, STDIN);
    io::close(saved_stdin_fd);

    // 5. Close our copy of the read end — only child has it now.
    io::close(read_fd);

    // 6. Write test messages through the pipe.
    let messages: &[&[u8]] = &[
        b"Hello from pipe!\n",
        b"Message 2\n",
        b"Message 3\n",
        b"Final message\n",
    ];

    for msg in messages {
        let n = io::write(write_fd, msg);
        if n < 0 {
            println!("[pipe-test] write error: {}", n);
            return 1;
        }
    }

    // 7. Close write end — signals EOF to consumer.
    io::close(write_fd);
    println!("[pipe-test] closed write end, waiting for consumer...");

    // 8. Wait for consumer to finish.
    let mut status: u64 = 0;
    let wait_ret = sys::waitpid(child_pid, Some(&mut status));
    if wait_ret < 0 {
        println!("[pipe-test] waitpid error: {}", wait_ret);
        return 1;
    }

    if status == 0 {
        println!("[pipe-test] PASS: pipe-consumer exited successfully");
        0
    } else {
        println!(
            "[pipe-test] FAIL: pipe-consumer exited with status {}",
            status
        );
        1
    }
}
