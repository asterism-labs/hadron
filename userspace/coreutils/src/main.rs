//! Hadron coreutils — busybox-style multi-call binary.
//!
//! Dispatches to a built-in command based on `argv[0]` (symlink name) or
//! the first argument when invoked as `coreutils <cmd>`.
//!
//! Supported commands: echo, cat, ls, uname, uptime, clear, true, false, yes.

#![no_std]
#![no_main]

use lepton_syslib::hadron_syscall::{
    DirEntryInfo, INODE_TYPE_CHARDEV, INODE_TYPE_DIR, INODE_TYPE_SYMLINK,
};
use lepton_syslib::io::{self, STDIN, STDOUT};
use lepton_syslib::{eprintln, print, println};

#[unsafe(no_mangle)]
pub extern "C" fn main(args: &[&str]) -> i32 {
    let program = args.first().copied().unwrap_or("coreutils");
    // Strip leading path to get the base command name.
    let cmd = program.rsplit('/').next().unwrap_or(program);

    // Handle `coreutils <subcmd> [args...]` invocation.
    let (cmd, arg_offset) = if cmd == "coreutils" && args.len() > 1 {
        (args[1], 2)
    } else {
        (cmd, 1)
    };
    let cmd_args = if arg_offset <= args.len() {
        &args[arg_offset..]
    } else {
        &[]
    };

    match cmd {
        "echo" => cmd_echo(cmd_args),
        "cat" => cmd_cat(cmd_args),
        "ls" => cmd_ls(cmd_args),
        "uname" => cmd_uname(),
        "uptime" => cmd_uptime(),
        "clear" => cmd_clear(),
        "true" => 0,
        "false" => 1,
        "yes" => cmd_yes(cmd_args),
        _ => {
            eprintln!("coreutils: unknown command: {}", cmd);
            127
        }
    }
}

/// `echo [-n] [args...]` — print arguments separated by spaces.
fn cmd_echo(args: &[&str]) -> i32 {
    let (suppress_newline, start) = if args.first() == Some(&"-n") {
        (true, 1)
    } else {
        (false, 0)
    };

    for (i, arg) in args[start..].iter().enumerate() {
        if i > 0 {
            print!(" ");
        }
        print!("{}", arg);
    }

    if !suppress_newline {
        println!();
    }
    0
}

/// `cat [files...]` — concatenate files to stdout. No args = copy stdin to stdout.
fn cmd_cat(args: &[&str]) -> i32 {
    if args.is_empty() {
        // Copy stdin to stdout.
        let mut buf = [0u8; 512];
        loop {
            let n = io::read(STDIN, &mut buf);
            if n <= 0 {
                break;
            }
            io::write(STDOUT, &buf[..n as usize]);
        }
        return 0;
    }

    let mut exit_code = 0;
    for path in args {
        let fd = io::open(path, 0);
        if fd < 0 {
            eprintln!("cat: {}: No such file or directory", path);
            exit_code = 1;
            continue;
        }
        let fd = fd as usize;

        let mut buf = [0u8; 512];
        loop {
            let n = io::read(fd, &mut buf);
            if n <= 0 {
                break;
            }
            io::write(STDOUT, &buf[..n as usize]);
        }

        io::close(fd);
    }
    exit_code
}

/// `ls [dir]` — list directory entries. Default is `/`.
fn cmd_ls(args: &[&str]) -> i32 {
    let path = args.first().copied().unwrap_or("/");

    let fd = io::open(path, 0);
    if fd < 0 {
        eprintln!("ls: cannot access '{}': No such file or directory", path);
        return 1;
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
        eprintln!("ls: cannot read directory '{}'", path);
        io::close(fd);
        return 1;
    }

    for entry in &entries[..count as usize] {
        let name_len = entry.name_len as usize;
        let name = core::str::from_utf8(&entry.name[..name_len]).unwrap_or("???");
        let type_char = if entry.inode_type == INODE_TYPE_DIR {
            'd'
        } else if entry.inode_type == INODE_TYPE_CHARDEV {
            'c'
        } else if entry.inode_type == INODE_TYPE_SYMLINK {
            'l'
        } else {
            '-'
        };
        println!("  {}  {}", type_char, name);
    }

    io::close(fd);
    0
}

/// `uname` — print kernel name and version.
fn cmd_uname() -> i32 {
    if let Some(ver) = lepton_syslib::sys::query_kernel_version() {
        let name_len = ver
            .name
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(ver.name.len());
        if let Ok(name) = core::str::from_utf8(&ver.name[..name_len]) {
            println!("{} {}.{}.{}", name, ver.major, ver.minor, ver.patch);
        } else {
            println!("{}.{}.{}", ver.major, ver.minor, ver.patch);
        }
    } else {
        eprintln!("uname: query failed");
        return 1;
    }
    0
}

/// `uptime` — print time since boot.
fn cmd_uptime() -> i32 {
    if let Some(uptime) = lepton_syslib::sys::query_uptime() {
        let secs = uptime.uptime_ns / 1_000_000_000;
        let ms = (uptime.uptime_ns % 1_000_000_000) / 1_000_000;
        println!("up {}.{:03}s", secs, ms);
    } else {
        eprintln!("uptime: query failed");
        return 1;
    }
    0
}

/// `clear` — clear terminal via ANSI escape.
fn cmd_clear() -> i32 {
    print!("\x1b[2J\x1b[H");
    0
}

/// `yes [string]` — repeatedly output a line. Default is "y".
fn cmd_yes(args: &[&str]) -> i32 {
    let text = args.first().copied().unwrap_or("y");
    loop {
        println!("{}", text);
    }
}
