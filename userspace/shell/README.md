# lsh

lsh (Lepton SHell) -- a minimal interactive shell for Hadron OS. lsh is a `no_std` userspace binary that provides command-line interaction with pipeline execution, I/O redirection, background job control, environment variable expansion, and PATH-based command resolution. It serves as the primary user interface when running Hadron interactively.

## Features

- **Pipelines** -- multi-stage command pipelines connected by `|`, supporting up to 8 stages with automatic pipe creation and teardown
- **I/O redirection** -- stdin redirect (`<`), stdout redirect with truncate (`>`) and append (`>>`), with file creation on write if the target does not exist
- **Background jobs** -- launch commands with `&`, track up to 16 background processes, and list them with the `jobs` built-in
- **Variable expansion** -- `$VAR` references in arguments are expanded from the environment at parse time
- **Double-quoted strings** -- preserve spaces and special characters inside quoted arguments
- **PATH-based command resolution** -- bare command names are searched in each `PATH` directory; absolute paths are used directly
- **Built-in commands** -- `cd` (with path normalization and `..` resolution), `export` (set/list environment variables), `exit`, `help`, `jobs`, and `sysinfo` (kernel version, memory, uptime)
- **Prompt with working directory** -- displays `hsh:<cwd>` with the current `PWD`
