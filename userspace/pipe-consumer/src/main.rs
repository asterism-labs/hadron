//! Pipe consumer child process.
//!
//! Reads from stdin (which the parent has redirected to a pipe read end
//! via dup2). Prints received data to stdout (still `/dev/console`).
//! Counts total bytes received. Exits when EOF (read returns 0).

#![no_std]
#![no_main]

use hadron_syslib::io::{self, STDIN, STDOUT};
use hadron_syslib::{println, sys};

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let pid = sys::getpid();
    println!("[pipe-consumer PID {}] reading from stdin...", pid);

    let mut total_bytes: usize = 0;
    let mut buf = [0u8; 256];

    loop {
        let n = io::read(STDIN, &mut buf);
        if n == 0 {
            // EOF — writer closed the pipe.
            break;
        }
        if n < 0 {
            println!("[pipe-consumer] read error: {}", n);
            return 1;
        }
        let n = n as usize;
        total_bytes += n;

        // Print received data to stdout.
        io::write(STDOUT, &buf[..n]);
    }

    println!(
        "[pipe-consumer PID {}] received {} bytes total",
        pid, total_bytes
    );
    0
}
