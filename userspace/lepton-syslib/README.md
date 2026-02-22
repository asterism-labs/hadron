# lepton-syslib

The userspace system library for Hadron OS. lepton-syslib is a `no_std` Rust crate that provides the foundational runtime for all userspace binaries: raw syscall wrappers, file and process I/O primitives, `print!`/`println!` macros, a heap allocator backed by kernel memory mapping, environment variable management, and the `_start` entry point that bootstraps argv/envp parsing before calling the user-defined `main`. It depends on `hadron-syscall` (with the `userspace` feature) for syscall number definitions and ABI types.

## Features

- **Syscall wrappers** (`sys` module) -- type-safe functions for `exit`, `getpid`, `spawn`, `waitpid`, `kill`, `dup2`, `pipe`, `mem_map`/`mem_unmap`, `clock_gettime`, and `query` calls (memory stats, uptime, kernel version)
- **File I/O** (`io` module) -- `open`, `read`, `write`, `close`, `stat`, and `readdir` over kernel file descriptors, plus `print!`, `println!`, `eprint!`, and `eprintln!` macros targeting stdout/stderr
- **Heap allocator** (`heap` module) -- a bump-with-freelist `GlobalAlloc` that grows in 64 KiB chunks via `sys_mem_map`, providing `alloc` support without an external allocator
- **Environment variables** (`env` module) -- `getenv`, `setenv`, `unsetenv`, and iteration backed by a `BTreeMap`, initialized from the envp array passed by the kernel at process startup
- **Process entry point** (`start` module) -- naked `_start` that reads argc/envc/argv/envp from the stack layout set up by the kernel, initializes the environment, and calls `extern "C" fn main(args: &[&str]) -> i32`
- **Panic handler** -- prints the panic message to stderr and exits the process with status 1
- **Process spawning with environment inheritance** -- `spawn` automatically serializes the current environment into the child process; `spawn_with_env` allows explicit environment control
