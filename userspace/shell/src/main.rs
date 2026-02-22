//! hsh — Hadron SHell.
//!
//! A minimal interactive shell supporting pipelines (`cmd1 | cmd2`),
//! background jobs (`cmd &`), double-quoted strings, I/O redirections
//! (`>`, `<`, `>>`), `$VAR` expansion, PATH-based command resolution,
//! `cd`, `export`, and built-in commands.

#![no_std]
#![no_main]

use lepton_syslib::io::{self, STDIN, STDOUT};
use lepton_syslib::{env, print, println, sys};

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
/// Path buffer size for command resolution.
const PATH_BUF_SIZE: usize = 128;
/// Expansion buffer size for $VAR expansion.
const EXPAND_BUF_SIZE: usize = 2048;

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
    /// `>` redirect stdout (truncate).
    RedirectOut,
    /// `>>` redirect stdout (append).
    RedirectAppend,
    /// `<` redirect stdin.
    RedirectIn,
}

/// Tokenize an input line into a fixed-size array.
///
/// Handles double-quoted strings (strips quotes, preserves spaces inside),
/// redirect operators (`>`, `>>`, `<`), pipes, and background `&`.
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

        // Redirect output.
        if bytes[i] == b'>' {
            if i + 1 < len && bytes[i + 1] == b'>' {
                tokens[count] = Token::RedirectAppend;
                i += 2;
            } else {
                tokens[count] = Token::RedirectOut;
                i += 1;
            }
            count += 1;
            continue;
        }

        // Redirect input.
        if bytes[i] == b'<' {
            tokens[count] = Token::RedirectIn;
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
        while i < len
            && bytes[i] != b' '
            && bytes[i] != b'\t'
            && bytes[i] != b'|'
            && bytes[i] != b'&'
            && bytes[i] != b'"'
            && bytes[i] != b'>'
            && bytes[i] != b'<'
        {
            i += 1;
        }
        tokens[count] = Token::Word(&input[start..i]);
        count += 1;
    }

    count
}

// ── $VAR expansion ──────────────────────────────────────────────────

/// Expand `$VAR` references in Word tokens.
///
/// Scans each Word token for `$NAME` patterns and replaces them with
/// the corresponding environment variable value. Expanded strings are
/// written into `expand_buf` and the token is updated to point there.
///
/// Returns the new offset into `expand_buf`.
fn expand_vars<'a>(
    tokens: &mut [Token<'a>; MAX_TOKENS],
    token_count: usize,
    expand_buf: &'a mut [u8; EXPAND_BUF_SIZE],
    mut offset: usize,
) -> usize {
    for i in 0..token_count {
        if let Token::Word(word) = tokens[i] {
            if !word.contains('$') {
                continue;
            }

            let start_offset = offset;
            let bytes = word.as_bytes();
            let len = bytes.len();
            let mut j = 0;

            while j < len {
                if bytes[j] == b'$' && j + 1 < len && is_var_start(bytes[j + 1]) {
                    // Read variable name.
                    j += 1;
                    let var_start = j;
                    while j < len && is_var_char(bytes[j]) {
                        j += 1;
                    }
                    let var_name = &word[var_start..j];

                    // Look up value.
                    if let Some(value) = env::getenv(var_name) {
                        let copy_len = value.len().min(EXPAND_BUF_SIZE - offset);
                        expand_buf[offset..offset + copy_len]
                            .copy_from_slice(&value.as_bytes()[..copy_len]);
                        offset += copy_len;
                    }
                    // If not found, the variable expands to empty string.
                } else {
                    if offset < EXPAND_BUF_SIZE {
                        expand_buf[offset] = bytes[j];
                        offset += 1;
                    }
                    j += 1;
                }
            }

            // Replace the token with the expanded string.
            // SAFETY: We only wrote valid UTF-8 bytes (from the original input
            // and env values which are valid UTF-8 strings). Each token's
            // expanded region is non-overlapping, so we use raw pointers to
            // avoid aliasing the mutable borrow of expand_buf.
            let expanded_len = offset - start_offset;
            let expanded = unsafe {
                let ptr = expand_buf.as_ptr().add(start_offset);
                core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, expanded_len))
            };
            tokens[i] = Token::Word(expanded);
        }
    }

    offset
}

/// Returns `true` if `b` can start a variable name (letter or underscore).
fn is_var_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Returns `true` if `b` can be part of a variable name (letter, digit, or underscore).
fn is_var_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

// ── Pipeline Stage ──────────────────────────────────────────────────

/// A single stage (command) in a pipeline.
struct Stage<'a> {
    /// Argument list (argv[0] is the command name).
    args: [&'a str; MAX_ARGS],
    /// Number of arguments.
    argc: usize,
    /// File to redirect stdin from (`<`).
    stdin_file: Option<&'a str>,
    /// File to redirect stdout to (`>`).
    stdout_file: Option<&'a str>,
    /// Whether stdout redirect is append mode (`>>`).
    stdout_append: bool,
}

impl<'a> Stage<'a> {
    const fn new() -> Self {
        Self {
            args: [""; MAX_ARGS],
            argc: 0,
            stdin_file: None,
            stdout_file: None,
            stdout_append: false,
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
///
/// Handles redirect tokens by storing the filename in the current stage.
fn parse<'a>(tokens: &[Token<'a>], token_count: usize) -> Pipeline<'a> {
    let mut pipeline = Pipeline {
        stages: [const { Stage::new() }; MAX_STAGES],
        stage_count: 1,
        background: false,
    };

    let mut stage_idx = 0;
    let mut i = 0;

    while i < token_count {
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
            Token::RedirectOut => {
                // Next token is the filename.
                i += 1;
                if i < token_count {
                    if let Token::Word(f) = tokens[i] {
                        pipeline.stages[stage_idx].stdout_file = Some(f);
                        pipeline.stages[stage_idx].stdout_append = false;
                    }
                }
            }
            Token::RedirectAppend => {
                i += 1;
                if i < token_count {
                    if let Token::Word(f) = tokens[i] {
                        pipeline.stages[stage_idx].stdout_file = Some(f);
                        pipeline.stages[stage_idx].stdout_append = true;
                    }
                }
            }
            Token::RedirectIn => {
                i += 1;
                if i < token_count {
                    if let Token::Word(f) = tokens[i] {
                        pipeline.stages[stage_idx].stdin_file = Some(f);
                    }
                }
            }
            Token::Word(w) => {
                let stage = &mut pipeline.stages[stage_idx];
                if stage.argc < MAX_ARGS {
                    stage.args[stage.argc] = w;
                    stage.argc += 1;
                }
            }
        }
        i += 1;
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

/// Resolve a command name to an absolute path using PATH.
///
/// Fills `path_buf` with the resolved path and returns it as a `&str`,
/// or `None` if the command was not found.
///
/// - Absolute paths (starting with `/`) are used as-is.
/// - Bare commands are searched in each PATH directory.
fn resolve_command<'a>(cmd: &str, path_buf: &'a mut [u8; PATH_BUF_SIZE]) -> Option<&'a str> {
    if cmd.starts_with('/') {
        // Absolute path — use as-is.
        let len = cmd.len().min(PATH_BUF_SIZE);
        path_buf[..len].copy_from_slice(&cmd.as_bytes()[..len]);
        // SAFETY: cmd is valid UTF-8, so the copied bytes are valid UTF-8.
        return Some(unsafe { core::str::from_utf8_unchecked(&path_buf[..len]) });
    }

    // Search PATH directories.
    let path_var = env::getenv("PATH").unwrap_or("/bin");
    let mut found_len: usize = 0;

    for dir in path_var.split(':') {
        if dir.is_empty() {
            continue;
        }
        // Construct dir/cmd in path_buf.
        let total = dir.len() + 1 + cmd.len();
        if total > PATH_BUF_SIZE {
            continue;
        }
        path_buf[..dir.len()].copy_from_slice(dir.as_bytes());
        path_buf[dir.len()] = b'/';
        path_buf[dir.len() + 1..total].copy_from_slice(cmd.as_bytes());

        // Check existence by trying to open the file.
        // The temporary &str is scoped to avoid borrow conflicts.
        let fd = {
            // SAFETY: dir and cmd are valid UTF-8, '/' is ASCII.
            let tmp = unsafe { core::str::from_utf8_unchecked(&path_buf[..total]) };
            io::open(tmp, 0)
        };
        if fd >= 0 {
            io::close(fd as usize);
            found_len = total;
            break;
        }
    }

    if found_len > 0 {
        // SAFETY: path_buf contains valid UTF-8 from the last successful iteration.
        Some(unsafe { core::str::from_utf8_unchecked(&path_buf[..found_len]) })
    } else {
        None
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
    // PGID for the pipeline: set to first child's PID.
    let mut pipeline_pgid: u32 = 0;

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

        // Handle stdin redirect from file.
        let stdin_redirect_fd = if let Some(file) = stage.stdin_file {
            let fd = io::open(file, 1); // READ
            if fd < 0 {
                println!("hsh: {}: No such file or directory", file);
                // Restore and skip this stage.
                if i > 0 {
                    sys::dup2(saved_stdin, STDIN);
                }
                continue;
            }
            sys::dup2(fd as usize, STDIN);
            Some(fd as usize)
        } else {
            None
        };

        // Redirect stdout to current pipe's write end.
        if i < n - 1 {
            sys::dup2(pipes[i].1, STDOUT);
        }

        // Handle stdout redirect to file.
        let stdout_redirect_fd = if let Some(file) = stage.stdout_file {
            let flags = if stage.stdout_append { 6 } else { 2 }; // WRITE | APPEND or WRITE
            let fd = io::open(file, flags);
            if fd < 0 {
                // Try to create the file — open with WRITE|CREATE flags.
                let fd2 = io::open(file, 10); // WRITE | CREATE
                if fd2 < 0 {
                    println!("hsh: {}: cannot open for writing", file);
                    // Restore and skip.
                    if i > 0 || stdin_redirect_fd.is_some() {
                        sys::dup2(saved_stdin, STDIN);
                    }
                    if i < n - 1 {
                        sys::dup2(saved_stdout, STDOUT);
                    }
                    if let Some(rfd) = stdin_redirect_fd {
                        io::close(rfd);
                    }
                    continue;
                }
                sys::dup2(fd2 as usize, STDOUT);
                Some(fd2 as usize)
            } else {
                sys::dup2(fd as usize, STDOUT);
                Some(fd as usize)
            }
        } else {
            None
        };

        // Resolve command path.
        let mut path_buf = [0u8; PATH_BUF_SIZE];
        let path = resolve_command(stage.args[0], &mut path_buf);

        let ret = if let Some(path) = path {
            // Build argv: argv[0] = resolved path, rest from user.
            let mut argv_buf: [&str; MAX_ARGS] = [""; MAX_ARGS];
            argv_buf[0] = path;
            let argv_count = stage.argc.min(MAX_ARGS);
            for j in 1..argv_count {
                argv_buf[j] = stage.args[j];
            }
            sys::spawn(path, &argv_buf[..argv_count])
        } else {
            -1
        };

        // Restore stdin/stdout immediately after spawn.
        if i > 0 || stdin_redirect_fd.is_some() {
            sys::dup2(saved_stdin, STDIN);
        }
        if i < n - 1 || stdout_redirect_fd.is_some() {
            sys::dup2(saved_stdout, STDOUT);
        }

        // Close redirect fds.
        if let Some(rfd) = stdin_redirect_fd {
            io::close(rfd);
        }
        if let Some(rfd) = stdout_redirect_fd {
            io::close(rfd);
        }

        if ret < 0 {
            println!("hsh: {}: command not found", stage.args[0]);
        } else {
            let child_pid = ret as u32;
            // First child becomes the pipeline's process group leader.
            if pipeline_pgid == 0 {
                pipeline_pgid = child_pid;
            }
            // Place all pipeline children in the same process group.
            sys::setpgid(child_pid, pipeline_pgid);
            child_pids[spawned] = child_pid;
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
            builtin_cd(args);
            true
        }
        "export" => {
            builtin_export(args);
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
    println!("  exit [code]      — exit the shell");
    println!("  help             — show this message");
    println!("  jobs             — list background jobs");
    println!("  sysinfo          — kernel version, memory, uptime");
    println!("  cd <dir>         — change working directory");
    println!("  export [VAR=val] — set/show environment variables");
    println!();
    println!("External commands (via PATH):");
    println!("  echo, cat, ls, uname, uptime, clear, true, false, yes, env, pwd");
    println!();
    println!("Syntax:");
    println!("  cmd1 | cmd2   — pipeline");
    println!("  cmd &         — run in background");
    println!("  cmd > file    — redirect stdout");
    println!("  cmd >> file   — append stdout");
    println!("  cmd < file    — redirect stdin");
    println!("  $VAR          — variable expansion");
    println!("  \"quoted arg\"  — preserve spaces");
}

/// `cd <dir>` — change working directory.
///
/// Validates the target exists (by opening and closing it), normalizes
/// the path, and updates the `PWD` environment variable.
fn builtin_cd(args: &[&str]) {
    let target = if args.len() > 1 {
        args[1]
    } else {
        // cd with no args goes to HOME.
        env::getenv("HOME").unwrap_or("/")
    };

    // Build absolute path.
    let mut abs_buf = [0u8; PATH_BUF_SIZE];
    let abs_path = if target.starts_with('/') {
        target
    } else {
        // Relative to current PWD.
        let pwd = env::getenv("PWD").unwrap_or("/");
        let total = if pwd == "/" {
            // /target
            let len = 1 + target.len();
            if len > PATH_BUF_SIZE {
                println!("hsh: cd: path too long");
                return;
            }
            abs_buf[0] = b'/';
            abs_buf[1..len].copy_from_slice(target.as_bytes());
            len
        } else {
            // pwd/target
            let len = pwd.len() + 1 + target.len();
            if len > PATH_BUF_SIZE {
                println!("hsh: cd: path too long");
                return;
            }
            abs_buf[..pwd.len()].copy_from_slice(pwd.as_bytes());
            abs_buf[pwd.len()] = b'/';
            abs_buf[pwd.len() + 1..len].copy_from_slice(target.as_bytes());
            len
        };
        // SAFETY: pwd and target are valid UTF-8, '/' is ASCII.
        unsafe { core::str::from_utf8_unchecked(&abs_buf[..total]) }
    };

    // Normalize the path (resolve `.` and `..`).
    let mut norm_buf = [0u8; PATH_BUF_SIZE];
    let normalized = normalize_path(abs_path, &mut norm_buf);

    // Validate the directory exists by trying to open it.
    let fd = io::open(normalized, 0);
    if fd < 0 {
        println!("hsh: cd: {}: No such file or directory", target);
        return;
    }
    io::close(fd as usize);

    env::setenv("PWD", normalized);
}

/// Normalize a path by resolving `.` and `..` components.
///
/// Writes the result into `buf` and returns a `&str` slice of it.
fn normalize_path<'a>(path: &str, buf: &'a mut [u8; PATH_BUF_SIZE]) -> &'a str {
    // Split by '/' and resolve components using a stack stored in buf.
    // We'll track component boundaries.
    let mut components: [usize; 64] = [0; 64]; // start indices in buf
    let mut comp_lens: [usize; 64] = [0; 64];
    let mut comp_count: usize = 0;
    let mut offset: usize = 0;

    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            // Pop the last component.
            if comp_count > 0 {
                comp_count -= 1;
                // Reset offset to the start of the popped component.
                offset = components[comp_count];
            }
            continue;
        }
        // Push component.
        if comp_count < 64 && offset + component.len() < PATH_BUF_SIZE {
            components[comp_count] = offset;
            comp_lens[comp_count] = component.len();
            buf[offset..offset + component.len()].copy_from_slice(component.as_bytes());
            offset += component.len();
            comp_count += 1;
        }
    }

    if comp_count == 0 {
        buf[0] = b'/';
        return unsafe { core::str::from_utf8_unchecked(&buf[..1]) };
    }

    // Rebuild path: /comp1/comp2/...
    let mut out_offset = 0;
    for i in 0..comp_count {
        if out_offset >= PATH_BUF_SIZE {
            break;
        }
        buf[out_offset] = b'/';
        out_offset += 1;
        let clen = comp_lens[i];
        let cstart = components[i];
        // Components may overlap in buf, so copy byte by byte.
        for j in 0..clen {
            if out_offset >= PATH_BUF_SIZE {
                break;
            }
            buf[out_offset] = buf[cstart + j];
            out_offset += 1;
        }
    }

    // SAFETY: We only wrote valid UTF-8 bytes (from path components and '/').
    unsafe { core::str::from_utf8_unchecked(&buf[..out_offset]) }
}

/// `export [VAR=value]` — set or list environment variables.
fn builtin_export(args: &[&str]) {
    if args.len() <= 1 {
        // No args: print all env vars.
        env::for_each(|key, value| {
            println!("{}={}", key, value);
        });
        return;
    }

    for &arg in &args[1..] {
        if let Some(eq_pos) = arg.find('=') {
            let key = &arg[..eq_pos];
            let value = &arg[eq_pos + 1..];
            env::setenv(key, value);
        } else {
            // export VAR (no =value) — just ensure it's exported (no-op for now).
        }
    }
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
    // Ignore SIGINT so Ctrl+C kills foreground children, not the shell itself.
    sys::signal(sys::SIGINT, sys::SIG_IGN);

    println!("hsh — Hadron SHell");
    println!("Type 'help' for available commands.\n");

    let mut jobs = JobTable::new();
    let mut line_buf = [0u8; LINE_BUF_SIZE];

    loop {
        // Show prompt with current directory.
        let cwd = env::getenv("PWD").unwrap_or("/");
        print!("hsh:{}> ", cwd);

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

        // Expand $VAR references.
        let mut expand_buf = [0u8; EXPAND_BUF_SIZE];
        expand_vars(&mut tokens, token_count, &mut expand_buf, 0);

        // Parse into pipeline.
        let pipeline = parse(&tokens, token_count);

        // Execute.
        execute(&pipeline, &mut jobs);
    }
}
