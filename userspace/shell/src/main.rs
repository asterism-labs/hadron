//! Hadron interactive shell.
//!
//! Provides a minimal shell with built-in commands: `help`, `echo`,
//! `sysinfo`, `ls`, `cat`, `pid`, `spawn`, `clear`, `exit`.

#![no_std]
#![no_main]

use hadron_syslib::hadron_syscall::{DirEntryInfo, INODE_TYPE_CHARDEV, INODE_TYPE_DIR};
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
    let (cmd, args) = match line.find(' ') {
        Some(pos) => (&line[..pos], line[pos + 1..].trim()),
        None => (line, ""),
    };

    match cmd {
        "help" => cmd_help(),
        "echo" => cmd_echo(args),
        "sysinfo" => cmd_sysinfo(),
        "ls" => cmd_ls(args),
        "cat" => cmd_cat(args),
        "pid" => cmd_pid(),
        "spawn" => cmd_spawn(args),
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
    println!("  ls      - List directory contents (default: /)");
    println!("  cat     - Print file contents");
    println!("  pid     - Show current process ID");
    println!("  spawn   - Spawn a child process (e.g. 'spawn /spinner')");
    println!("  clear   - Clear the screen");
    println!("  exit    - Exit the shell");
}

fn cmd_echo(args: &str) {
    println!("{}", args);
}

fn cmd_sysinfo() {
    if let Some(ver) = hadron_syslib::sys::query_kernel_version() {
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

    if let Some(uptime) = hadron_syslib::sys::query_uptime() {
        let secs = uptime.uptime_ns / 1_000_000_000;
        let ms = (uptime.uptime_ns % 1_000_000_000) / 1_000_000;
        println!("Uptime:  {}.{:03}s", secs, ms);
    } else {
        println!("Uptime:  (query failed)");
    }
}

fn cmd_ls(args: &str) {
    let path = if args.is_empty() { "/" } else { args };

    let fd = io::open(path, 0);
    if fd < 0 {
        println!("ls: cannot open '{}': no such file or directory", path);
        return;
    }
    let fd = fd as usize;

    let mut entries = [DirEntryInfo {
        inode_type: 0,
        name_len: 0,
        _pad: [0; 2],
        name: [0; 60],
    }; 32];

    let count = io::readdir(fd, &mut entries);
    if count < 0 {
        println!("ls: cannot read directory '{}'", path);
        io::close(fd);
        return;
    }

    for entry in &entries[..count as usize] {
        let name_len = entry.name_len as usize;
        let name = core::str::from_utf8(&entry.name[..name_len]).unwrap_or("???");
        let type_char = if entry.inode_type == INODE_TYPE_DIR {
            'd'
        } else if entry.inode_type == INODE_TYPE_CHARDEV {
            'c'
        } else {
            '-'
        };
        println!("  {}  {}", type_char, name);
    }

    io::close(fd);
}

fn cmd_cat(args: &str) {
    if args.is_empty() {
        println!("cat: missing file path");
        return;
    }

    let fd = io::open(args, 0);
    if fd < 0 {
        println!("cat: cannot open '{}': no such file or directory", args);
        return;
    }
    let fd = fd as usize;

    let mut buf = [0u8; 512];
    loop {
        let n = io::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        let n = n as usize;
        if let Ok(s) = core::str::from_utf8(&buf[..n]) {
            print!("{}", s);
        } else {
            println!("(binary data: {} bytes)", n);
            break;
        }
    }

    io::close(fd);
}

fn cmd_spawn(args: &str) {
    if args.is_empty() {
        println!("spawn: missing path (e.g. 'spawn /spinner')");
        return;
    }

    let ret = hadron_syslib::sys::spawn(args);
    if ret < 0 {
        println!("spawn: failed to spawn '{}' (error {})", args, ret);
        return;
    }

    let child_pid = ret as u32;
    println!("spawned child PID {}", child_pid);

    let mut status: u64 = 0;
    let wait_ret = hadron_syslib::sys::waitpid(child_pid, Some(&mut status));
    if wait_ret < 0 {
        println!("waitpid: error {}", wait_ret);
    } else {
        println!("child {} exited with status {}", wait_ret, status);
    }
}

fn cmd_pid() {
    let pid = hadron_syslib::sys::getpid();
    println!("PID: {}", pid);
}

fn cmd_clear() {
    for _ in 0..50 {
        println!();
    }
}

fn cmd_exit() {
    println!("Goodbye!");
    hadron_syslib::sys::exit(0);
}
