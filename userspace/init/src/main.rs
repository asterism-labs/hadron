//! Hadron init process — interactive shell.
//!
//! Runs as PID 1 in userspace (ring 3). Provides a minimal echo shell with
//! built-in commands: `help`, `echo`, `sysinfo`, `clear`, `exit`.

#![no_std]
#![no_main]

use hadron_syslib::io::{self, STDIN};
use hadron_syslib::{print, println};

/// Read buffer size for stdin reads.
const READ_BUF_SIZE: usize = 256;

/// Line buffer for accumulating a complete command.
const LINE_BUF_SIZE: usize = 256;

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    println!("Hadron OS v0.1.0");
    println!("Type 'help' for available commands.\n");

    let mut line_buf = [0u8; LINE_BUF_SIZE];
    loop {
        print!("> ");
        let len = read_line(&mut line_buf);
        let line = match core::str::from_utf8(&line_buf[..len]) {
            Ok(s) => s.trim(),
            Err(_) => {
                println!("error: invalid UTF-8 input");
                continue;
            }
        };

        if line.is_empty() {
            continue;
        }

        dispatch_command(line);
    }
}

/// Read a complete line from stdin into `buf`, returning the number of bytes
/// (excluding the trailing newline).
///
/// The kernel performs cooked-mode line editing, so `read()` returns a full
/// line including the `\n` terminator.
fn read_line(buf: &mut [u8]) -> usize {
    let mut total = 0;
    let mut read_buf = [0u8; READ_BUF_SIZE];

    loop {
        let n = io::read(STDIN, &mut read_buf);
        if n <= 0 {
            break;
        }
        let n = n as usize;

        for i in 0..n {
            if read_buf[i] == b'\n' {
                return total;
            }
            if total < buf.len() {
                buf[total] = read_buf[i];
                total += 1;
            }
        }
    }

    total
}

/// Parse and dispatch a command line.
fn dispatch_command(line: &str) {
    // Split into command and arguments at the first space.
    let (cmd, args) = match line.find(' ') {
        Some(pos) => (&line[..pos], line[pos + 1..].trim()),
        None => (line, ""),
    };

    match cmd {
        "help" => cmd_help(),
        "echo" => cmd_echo(args),
        "sysinfo" => cmd_sysinfo(),
        "clear" => cmd_clear(),
        "exit" => cmd_exit(),
        _ => println!("unknown command: {}", cmd),
    }
}

fn cmd_help() {
    println!("Available commands:");
    println!("  help    - Show this help message");
    println!("  echo    - Print arguments to stdout");
    println!("  sysinfo - Display kernel version, memory, and uptime");
    println!("  clear   - Clear the screen");
    println!("  exit    - Exit the shell");
}

fn cmd_echo(args: &str) {
    println!("{}", args);
}

fn cmd_sysinfo() {
    // Kernel version.
    if let Some(ver) = hadron_syslib::sys::query_kernel_version() {
        // Extract the name, trimming NUL padding.
        let name_len = ver
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(ver.name.len());
        if let Ok(name) = core::str::from_utf8(&ver.name[..name_len]) {
            println!(
                "Kernel:  {} v{}.{}.{}",
                name, ver.major, ver.minor, ver.patch
            );
        } else {
            println!("Kernel:  v{}.{}.{}", ver.major, ver.minor, ver.patch);
        }
    } else {
        println!("Kernel:  (query failed)");
    }

    // Memory stats.
    if let Some(mem) = hadron_syslib::sys::query_memory() {
        let total_kb = mem.total_bytes / 1024;
        let free_kb = mem.free_bytes / 1024;
        let used_kb = mem.used_bytes / 1024;
        println!(
            "Memory:  {} KiB total, {} KiB used, {} KiB free",
            total_kb, used_kb, free_kb
        );
    } else {
        println!("Memory:  (query failed)");
    }

    // Uptime.
    if let Some(uptime) = hadron_syslib::sys::query_uptime() {
        let secs = uptime.uptime_ns / 1_000_000_000;
        let ms = (uptime.uptime_ns % 1_000_000_000) / 1_000_000;
        println!("Uptime:  {}.{:03}s", secs, ms);
    } else {
        println!("Uptime:  (query failed)");
    }
}

fn cmd_clear() {
    // Print enough newlines to push content off a typical terminal.
    for _ in 0..50 {
        println!();
    }
}

fn cmd_exit() {
    println!("Goodbye!");
    hadron_syslib::sys::exit(0);
}
