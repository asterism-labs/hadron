# lepton-coreutils

A busybox-style multi-call binary providing core command-line utilities for Hadron OS. lepton-coreutils dispatches to the appropriate command based on `argv[0]` (typically set via symlinks in `/bin`), or when invoked directly as `coreutils <cmd>`. It is a `no_std` binary built on `lepton-syslib`.

## Features

- **Multi-call dispatch** -- resolves the command name from `argv[0]` (stripping any leading path), so a single binary can serve all utilities through symlinks
- **echo** -- print arguments to stdout, with `-n` flag to suppress the trailing newline
- **cat** -- concatenate files to stdout, or copy stdin to stdout when invoked with no arguments
- **ls** -- list directory entries with type indicators (`d` for directory, `c` for character device, `l` for symlink, `-` for regular file)
- **uname** -- print kernel name and version by querying the kernel version syscall
- **uptime** -- print time since boot in seconds with millisecond precision
- **clear** -- clear the terminal using ANSI escape sequences
- **env** -- print all environment variables in `KEY=value` format
- **pwd** -- print the current working directory from the `PWD` environment variable
- **yes** -- repeatedly print a string (default `"y"`) to stdout
- **true / false** -- exit with status 0 or 1 respectively
