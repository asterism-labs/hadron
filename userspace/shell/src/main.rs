//! hsh — Hadron SHell.
//!
//! A minimal interactive shell supporting pipelines (`cmd1 | cmd2`),
//! background jobs (`cmd &`), double-quoted strings, and built-in commands.
//!
//! All storage is stack-allocated (no heap in userspace).

#![no_std]
#![no_main]

use lepton_syslib::io::{self, STDIN, STDOUT};
use lepton_syslib::{print, println, sys};

// ── Constants ───────────────────────────────────────────────────────

/// Maximum tokens per input line.
const MAX_TOKENS: usize = 64;
/// Maximum arguments per pipeline stage.
const MAX_ARGS: usize = 16;
/// Maximum pipeline stages (commands separated by `|`).
const MAX_STAGES: usize = 8;
/// Maximum tracked background jobs.
const MAX_JOBS: usize = 16;
/// Input line buffer size.
const LINE_BUF_SIZE: usize = 256;
/// Stdin read chunk size.
const READ_BUF_SIZE: usize = 256;

// ── Token ───────────────────────────────────────────────────────────

/// A token from the input line.
#[derive(Clone, Copy)]
enum Token<'a> {
    /// A word (command name or argument).
    Word(&'a str),
    /// `|` pipe operator.
    Pipe,
    /// `&` background operator.
    Background,
}

/// Tokenize an input line into a fixed-size array.
///
/// Handles double-quoted strings (strips quotes, preserves spaces inside).
/// Returns the number of tokens produced.
fn tokenize<'a>(input: &'a str, tokens: &mut [Token<'a>; MAX_TOKENS]) -> usize {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut count = 0;
    let mut i = 0;

    while i < len && count < MAX_TOKENS {
        // Skip whitespace.
        if bytes[i] == b' ' || bytes[i] == b'\t' {
            i += 1;
            continue;
        }

        // Pipe.
        if bytes[i] == b'|' {
            tokens[count] = Token::Pipe;
            count += 1;
            i += 1;
            continue;
        }

        // Background.
        if bytes[i] == b'&' {
            tokens[count] = Token::Background;
            count += 1;
            i += 1;
            continue;
        }

        // Double-quoted string.
        if bytes[i] == b'"' {
            i += 1; // skip opening quote
            let start = i;
            while i < len && bytes[i] != b'"' {
                i += 1;
            }
            // SAFETY: start..i is within the valid UTF-8 input string.
            let word = &input[start..i];
            if i < len {
                i += 1; // skip closing quote
            }
            tokens[count] = Token::Word(word);
            count += 1;
            continue;
        }

        // Unquoted word.
        let start = i;
        while i < len && bytes[i] != b' ' && bytes[i] != b'\t' && bytes[i] != b'|' && bytes[i] != b'&' && bytes[i] != b'"' {
            i += 1;
        }
        tokens[count] = Token::Word(&input[start..i]);
        count += 1;
    }

    count
}

// ── Pipeline Stage ──────────────────────────────────────────────────

/// A single stage (command) in a pipeline.
struct Stage<'a> {
    /// Argument list (argv[0] is the command name).
    args: [&'a str; MAX_ARGS],
    /// Number of arguments.
    argc: usize,
}

impl<'a> Stage<'a> {
    const fn new() -> Self {
        Self {
            args: [""; MAX_ARGS],
            argc: 0,
        }
    }
}

/// Parsed pipeline: stages + whether it runs in the background.
struct Pipeline<'a> {
    stages: [Stage<'a>; MAX_STAGES],
    stage_count: usize,
    background: bool,
}

/// Parse tokens into a pipeline.
fn parse<'a>(tokens: &[Token<'a>], token_count: usize) -> Pipeline<'a> {
    let mut pipeline = Pipeline {
        stages: [const { Stage::new() }; MAX_STAGES],
        stage_count: 1,
        background: false,
    };

    let mut stage_idx = 0;

    for i in 0..token_count {
        match tokens[i] {
            Token::Pipe => {
                stage_idx += 1;
                if stage_idx >= MAX_STAGES {
                    println!("hsh: too many pipeline stages");
                    return pipeline;
                }
                pipeline.stage_count = stage_idx + 1;
            }
            Token::Background => {
                pipeline.background = true;
            }
            Token::Word(w) => {
                let stage = &mut pipeline.stages[stage_idx];
                if stage.argc < MAX_ARGS {
                    stage.args[stage.argc] = w;
                    stage.argc += 1;
                }
            }
        }
    }

    pipeline
}

// ── Job Tracking ────────────────────────────────────────────────────

/// A background job entry.
struct Job {
    /// Whether this slot is active.
    active: bool,
    /// Process ID of the background job.
    pid: u32,
    /// Command name (truncated to fit).
    cmd: [u8; 64],
    /// Command name length.
    cmd_len: usize,
}

impl Job {
    const fn empty() -> Self {
        Self {
            active: false,
            pid: 0,
            cmd: [0; 64],
            cmd_len: 0,
        }
    }
}

/// Background job table.
struct JobTable {
    jobs: [Job; MAX_JOBS],
}

impl JobTable {
    const fn new() -> Self {
        Self {
            jobs: [const { Job::empty() }; MAX_JOBS],
        }
    }

    /// Add a job. Returns the job number (1-based) or 0 if full.
    fn add(&mut self, pid: u32, cmd: &str) -> usize {
        for (i, slot) in self.jobs.iter_mut().enumerate() {
            if !slot.active {
                slot.active = true;
                slot.pid = pid;
                let copy_len = cmd.len().min(64);
                slot.cmd[..copy_len].copy_from_slice(&cmd.as_bytes()[..copy_len]);
                slot.cmd_len = copy_len;
                return i + 1;
            }
        }
        0
    }

    /// List all active jobs.
    fn list(&self) {
        for (i, job) in self.jobs.iter().enumerate() {
            if job.active {
                let cmd = core::str::from_utf8(&job.cmd[..job.cmd_len]).unwrap_or("???");
                println!("[{}] {} {}", i + 1, job.pid, cmd);
            }
        }
    }
}

// ── Command Resolution ──────────────────────────────────────────────

/// Resolve a command name to an absolute path.
///
/// Fills `path_buf` with the resolved path and returns it as a `&str`.
/// If the command already starts with `/`, uses it as-is.
/// Otherwise prepends `/` (e.g., `echo` → `/echo`).
fn resolve_command<'a>(cmd: &str, path_buf: &'a mut [u8; 128]) -> &'a str {
    if cmd.starts_with('/') {
        // Already absolute.
        let len = cmd.len().min(128);
        path_buf[..len].copy_from_slice(&cmd.as_bytes()[..len]);
        // SAFETY: cmd is valid UTF-8, so the copied bytes are valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(&path_buf[..len]) }
    } else {
        // Prepend '/'.
        path_buf[0] = b'/';
        let len = cmd.len().min(127);
        path_buf[1..1 + len].copy_from_slice(&cmd.as_bytes()[..len]);
        // SAFETY: '/' + valid UTF-8 = valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(&path_buf[..1 + len]) }
    }
}

// ── Pipeline Execution ──────────────────────────────────────────────

/// Execute a parsed pipeline.
fn execute(pipeline: &Pipeline<'_>, jobs: &mut JobTable) {
    let n = pipeline.stage_count;

    // Check if the first (and only) stage is a built-in.
    if n == 1 && pipeline.stages[0].argc > 0 {
        let stage = &pipeline.stages[0];
        if execute_builtin(stage.args[0], &stage.args[..stage.argc], jobs) {
            return;
        }
    }

    // Collect PIDs for waitpid.
    let mut child_pids = [0u32; MAX_STAGES];
    let mut spawned = 0;

    // Save original stdin/stdout by opening /dev/console.
    let saved_stdin = io::open("/dev/console", 1); // READ = 1
    let saved_stdout = io::open("/dev/console", 2); // WRITE = 2
    if saved_stdin < 0 || saved_stdout < 0 {
        println!("hsh: failed to save stdin/stdout");
        return;
    }
    let saved_stdin = saved_stdin as usize;
    let saved_stdout = saved_stdout as usize;

    // Create N-1 pipes.
    let mut pipes = [(0usize, 0usize); MAX_STAGES]; // (read_fd, write_fd)
    for i in 0..n.saturating_sub(1) {
        match sys::pipe() {
            Ok(fds) => pipes[i] = fds,
            Err(e) => {
                println!("hsh: pipe() failed: {}", e);
                io::close(saved_stdin);
                io::close(saved_stdout);
                return;
            }
        }
    }

    // Spawn each stage.
    for i in 0..n {
        let stage = &pipeline.stages[i];
        if stage.argc == 0 {
            continue;
        }

        // Redirect stdin from previous pipe's read end.
        if i > 0 {
            sys::dup2(pipes[i - 1].0, STDIN);
        }

        // Redirect stdout to current pipe's write end.
        if i < n - 1 {
            sys::dup2(pipes[i].1, STDOUT);
        }

        // Resolve command path.
        let mut path_buf = [0u8; 128];
        let path = resolve_command(stage.args[0], &mut path_buf);

        // Build argv: the args as written (args[0] is what the user typed,
        // but for symlink dispatch we want argv[0] to be the path).
        let mut argv_buf: [&str; MAX_ARGS] = [""; MAX_ARGS];
        argv_buf[0] = path;
        let argv_count = stage.argc.min(MAX_ARGS);
        for j in 1..argv_count {
            argv_buf[j] = stage.args[j];
        }

        let ret = sys::spawn(path, &argv_buf[..argv_count]);

        // Restore stdin/stdout immediately after spawn.
        if i > 0 {
            sys::dup2(saved_stdin, STDIN);
        }
        if i < n - 1 {
            sys::dup2(saved_stdout, STDOUT);
        }

        if ret < 0 {
            println!("hsh: {}: command not found", stage.args[0]);
        } else {
            child_pids[spawned] = ret as u32;
            spawned += 1;
        }
    }

    // Close all pipe fds in the parent.
    for i in 0..n.saturating_sub(1) {
        io::close(pipes[i].0);
        io::close(pipes[i].1);
    }

    // Close saved fds.
    io::close(saved_stdin);
    io::close(saved_stdout);

    if pipeline.background {
        // Record background jobs.
        for &pid in &child_pids[..spawned] {
            let cmd = pipeline.stages[0].args[0];
            let job_num = jobs.add(pid, cmd);
            if job_num > 0 {
                println!("[{}] {}", job_num, pid);
            }
        }
    } else {
        // Wait for all children.
        for &pid in &child_pids[..spawned] {
            let mut status: u64 = 0;
            sys::waitpid(pid, Some(&mut status));
        }
    }
}

// ── Built-in Commands ───────────────────────────────────────────────

/// Try to execute a built-in command. Returns `true` if handled.
fn execute_builtin(cmd: &str, args: &[&str], jobs: &mut JobTable) -> bool {
    match cmd {
        "exit" => builtin_exit(args),
        "help" => {
            builtin_help();
            true
        }
        "jobs" => {
            jobs.list();
            true
        }
        "sysinfo" => {
            builtin_sysinfo();
            true
        }
        "cd" => {
            // No-op: we don't have a working directory concept yet.
            println!("hsh: cd: not supported");
            true
        }
        _ => false,
    }
}

fn builtin_exit(args: &[&str]) -> bool {
    let code = if args.len() > 1 {
        parse_usize(args[1]).unwrap_or(0)
    } else {
        0
    };
    sys::exit(code);
}

fn builtin_help() {
    println!("hsh — Hadron SHell");
    println!();
    println!("Built-in commands:");
    println!("  exit [code]  — exit the shell");
    println!("  help         — show this message");
    println!("  jobs         — list background jobs");
    println!("  sysinfo      — kernel version, memory, uptime");
    println!();
    println!("External commands (via /coreutils symlinks):");
    println!("  echo, cat, ls, uname, uptime, clear, true, false, yes");
    println!();
    println!("Syntax:");
    println!("  cmd1 | cmd2  — pipeline");
    println!("  cmd &        — run in background");
    println!("  \"quoted arg\" — preserve spaces");
}

fn builtin_sysinfo() {
    if let Some(ver) = sys::query_kernel_version() {
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
    }

    if let Some(mem) = sys::query_memory() {
        let total_kb = mem.total_bytes / 1024;
        let free_kb = mem.free_bytes / 1024;
        let used_kb = mem.used_bytes / 1024;
        println!(
            "Memory:  {} KiB total, {} KiB used, {} KiB free",
            total_kb, used_kb, free_kb
        );
    }

    if let Some(uptime) = sys::query_uptime() {
        let secs = uptime.uptime_ns / 1_000_000_000;
        let ms = (uptime.uptime_ns % 1_000_000_000) / 1_000_000;
        println!("Uptime:  {}.{:03}s", secs, ms);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Parse a decimal string to usize.
fn parse_usize(s: &str) -> Option<usize> {
    let mut result: usize = 0;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(result)
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

// ── Main ────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    println!("hsh — Hadron SHell");
    println!("Type 'help' for available commands.\n");

    let mut jobs = JobTable::new();
    let mut line_buf = [0u8; LINE_BUF_SIZE];

    loop {
        print!("hsh> ");

        let len = read_line(&mut line_buf);
        let line = match core::str::from_utf8(&line_buf[..len]) {
            Ok(s) => s.trim(),
            Err(_) => {
                println!("hsh: invalid UTF-8 input");
                continue;
            }
        };

        if line.is_empty() {
            continue;
        }

        // Tokenize.
        let mut tokens = [Token::Word(""); MAX_TOKENS];
        let token_count = tokenize(line, &mut tokens);
        if token_count == 0 {
            continue;
        }

        // Parse into pipeline.
        let pipeline = parse(&tokens, token_count);

        // Execute.
        execute(&pipeline, &mut jobs);
    }
}
