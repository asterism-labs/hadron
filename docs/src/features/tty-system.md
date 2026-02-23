# TTY & Terminal System

**Status: Completed** (implemented in commit 7dc9ede)

Hadron provides a comprehensive virtual terminal (TTY) subsystem with cooked-mode line editing, multi-VT support (6 virtual terminals), and signal dispatch to foreground process groups. The TTY layer integrates with the VFS to provide `/dev/console` and `/dev/ttyN` character device nodes.

Source: [`kernel/hadron-kernel/src/tty/`](https://github.com/anomalyco/hadron/blob/main/kernel/hadron-kernel/src/tty/), [`kernel/hadron-kernel/src/tty/ldisc.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/hadron-kernel/src/tty/ldisc.rs)

## Architecture

The TTY subsystem consists of:

1. **Virtual Terminals (VTs)** -- 6 independent TTY instances (Alt+F1-F6 switching)
2. **Line Discipline** -- Cooked-mode input processing with line editing
3. **VFS Integration** -- `/dev/console` and `/dev/ttyN` character device nodes
4. **Signal Dispatch** -- Signal delivery to foreground process groups (SIGINT, SIGTSTP)

### Key Types

| Type | Role |
|------|------|
| `Tty` | Virtual terminal instance; owns line discipline, foreground PID, waker |
| `LineDiscipline` | Cooked-mode input processing with editing and signal dispatch |
| `DevConsole` | VFS inode for `/dev/console` (active VT); delegates to the active TTY |
| `DevTty` | VFS inode for `/dev/ttyN` (specific VT); direct access to a terminal |

## Cooked-Mode Line Editing

The line discipline processes input in cooked mode (canonical mode), providing line-based editing:

### Supported Control Characters

| Key | Behavior |
|-----|----------|
| **Ctrl+H** / **Backspace** | Delete last character (destructive: outputs `BS SP BS`) |
| **Ctrl+U** | Clear entire line (^U) |
| **Ctrl+D** | EOF marker (if at line start, close input; otherwise ignore) |
| **Ctrl+C** | Send SIGINT to foreground process group |
| **Ctrl+Z** | Send SIGTSTP to foreground process group |
| **Ctrl+\** | Send SIGQUIT to foreground process group |
| **Enter** (Ctrl+M) | Submit line; echo CR LF |

### Raw Mode Support

Although raw mode (`~ICANON`) can be set via `termios` ioctls, the line discipline currently ignores it and always operates in cooked mode. **Known limitation**: Interactive programs like `vi`, `nano`, and `less` that require character-at-a-time input do not work correctly. This is listed in [Known Issues](../reference/known-issues.md).

## Multi-VT Support

The system supports 6 virtual terminals (VT 0-5), selectable via:

- **Alt+F1** -- Switch to VT 0
- **Alt+F2** -- Switch to VT 1
- **Alt+F3** -- Switch to VT 2
- **Alt+F4** -- Switch to VT 3
- **Alt+F5** -- Switch to VT 4
- **Alt+F6** -- Switch to VT 5

Each VT is independent:

- **Independent process groups** -- Each VT has its own foreground process group ID.
- **Independent line buffers** -- Each VT maintains its own input line and output scrollback.
- **Independent state** -- Terminal size, attributes, and keyboard repeat settings per VT.

**Implementation:** A global `ACTIVE_VT` atomic tracks the currently displayed VT (0-5). The keyboard interrupt handler checks Alt+Fn key combinations and updates `ACTIVE_VT`. `/dev/console` reads and writes are dispatched to the active VT; `/dev/ttyN` nodes directly access VT N.

## Keyboard Integration

Keyboard input flows through the interrupt handler:

1. **PS/2 keyboard interrupt** → raw scancode in `SCANCODE_BUF`.
2. **Scancode processing** → Convert extended scancodes (0xE0 prefix) and modifier keys.
3. **VT multiplexing** → Check for Alt+Fn (VT switch) or Ctrl+Alt+Del (reboot); otherwise dispatch to active VT.
4. **Line discipline** → Process input character through the active TTY's line discipline.
5. **Waker notification** → Wake any tasks blocked on `/dev/console` reads.

## Signal Dispatch

When a process group is set as the foreground process group for a TTY and a signal character (Ctrl+C, Ctrl+Z, Ctrl+\) is pressed, the line discipline sends the corresponding signal to the entire foreground process group:

- **Ctrl+C** → SIGINT
- **Ctrl+Z** → SIGTSTP
- **Ctrl+\** → SIGQUIT

Process groups are managed via `sys_task_setpgid()` and `sys_task_getpgid()` syscalls. The kernel tracks foreground process group in the TTY structure.

## VFS Integration

### `/dev/console`

The `/dev/console` device node provides access to the **active** virtual terminal (whichever one is currently displayed). Reads and writes are dispatched to the active TTY dynamically:

- **Read**: Blocks until a complete line is entered (Ctrl+M or Ctrl+D), then returns the line.
- **Write**: Outputs to the active TTY's frame buffer (via the TTY layer to the graphics device).

### `/dev/ttyN`

The `/dev/ttyN` device nodes (where N is 0-5) provide direct access to a specific VT:

- **Read**: Blocks until a complete line is entered on VT N.
- **Write**: Outputs to VT N regardless of which VT is currently active.

## Implementation Status

Virtual terminal multiplexing (6 VTs, Alt+F1-F6 switching)
Cooked-mode line editing (Ctrl+H, Ctrl+U, Ctrl+D, Enter)
Signal character dispatch (Ctrl+C, Ctrl+Z, Ctrl+\)
`/dev/console` and `/dev/ttyN` character devices
Keyboard integration
Per-VT foreground process group tracking
Raw mode (`~ICANON`) support
TTY size (TIOCGWINSZ) support
Flow control (Ctrl+S / Ctrl+Q)

## Files to Modify

- `kernel/hadron-kernel/src/tty/mod.rs` -- TTY and VT management
- `kernel/hadron-kernel/src/tty/ldisc.rs` -- Line discipline implementation
- `kernel/hadron-kernel/src/tty/device.rs` -- VFS inode implementations for `/dev/console` and `/dev/ttyN`
- `kernel/hadron-kernel/src/fs/devfs.rs` -- Registration of TTY device nodes in devfs

## References

- **Architecture**: [I/O & Filesystem](../architecture/io-filesystem.md#tty-layer)
- **Synchronization**: [Synchronization & IPC](../architecture/sync-ipc.md)
- **Known Issues**: [Known Issues](../reference/known-issues.md#pty-line-discipline-does-not-honor-icanon)
