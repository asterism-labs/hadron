# Input Handling

## Status

**Completed** — Raw PS/2 keyboard and mouse devices exposed via devfs with direct event reading.

## Overview

Input Handling exposes raw keyboard and mouse events to userspace programs via `/dev/kbd` and `/dev/mouse`. Unlike the TTY line discipline (which cooked keystrokes), these device nodes provide direct hardware-level events: PS/2 scancodes and mouse movement vectors. This is essential for games, compositors, and any application needing exclusive input control.

## Key Types

| Type | Module | Purpose |
|------|--------|---------|
| `AsyncKeyboard` | `drivers::ps2_keyboard` | PS/2 keyboard driver; produces scancodes |
| `AsyncMouse` | `drivers::ps2_mouse` | PS/2 mouse driver; produces movement and button events |
| `KeyEvent` | `syscall::input` | Raw keyboard event (scancode + pressed/released) |
| `MouseEvent` | `syscall::input` | Raw mouse event (dx, dy, button state) |
| `KbdDeviceInode` | `fs::devfs` | VFS inode for `/dev/kbd` |
| `MouseDeviceInode` | `fs::devfs` | VFS inode for `/dev/mouse` |

## Design Decisions

### Two Input Streams: TTY and Raw Devices

Hadron maintains two independent input streams:

1. **TTY line discipline** (`/dev/ttyN`): Cooked input with echo, line editing, and signal delivery (Ctrl+C → SIGINT).
2. **Raw devices** (`/dev/kbd`, `/dev/mouse`): Direct hardware events; no processing.

This separation allows:
- Normal shell programs to use the TTY (familiar POSIX behavior)
- Games/compositors to read raw events without interfering with shells
- Each process to independently choose which input it consumes

### Event-Based Reading with Blocking Semantics

Reading from `/dev/kbd` or `/dev/mouse` is async (at the kernel level) but exposes blocking semantics to userspace:

```rust
async fn read(&self, offset: usize, buf: &mut [u8]) 
    -> Result<usize, FsError> 
{
    let event = self.keyboard.next_event().await;
    let bytes = event.as_bytes();
    // ... copy to user buffer ...
    Ok(bytes.len())
}
```

The VFS bridge ensures that `read()` blocks until an event is available, matching standard POSIX file I/O.

### Why Separate Keyboard and Mouse?

- **Simplicity**: Each device has a single responsibility (keystroke vs movement).
- **Multiplexing**: Multiple readers can coexist on `/dev/mouse` (e.g., game + status daemon).
- **Protocol neutrality**: Future input devices (joystick, touchpad) fit naturally.

## Implementation Status

### Completed

- `/dev/kbd` device node for raw keyboard input
- `/dev/mouse` device node for raw mouse input
- PS/2 keyboard driver integration
- PS/2 mouse driver integration
- Blocking read semantics via VFS

### Design Pattern: Async Bridge

The devfs inodes bridge async drivers to the synchronous VFS interface:

```rust
struct KbdDeviceInode {
    keyboard: Arc<AsyncKeyboard>,
}

impl Inode for KbdDeviceInode {
    fn read<'a>(&'a self, offset: usize, buf: &'a mut [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>
    {
        Box::pin(async move {
            let event = self.keyboard.next_event().await;
            // ... copy event to buf ...
        })
    }
}
```

This pattern allows the driver to expose raw async behavior while users see standard blocking file I/O.

## Files to Reference

- **PS/2 keyboard driver**: [`kernel/kernel/src/drivers/ps2_keyboard.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/drivers/ps2_keyboard.rs)
- **PS/2 mouse driver**: [`kernel/kernel/src/drivers/ps2_mouse.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/drivers/ps2_mouse.rs)
- **devfs integration**: [`kernel/kernel/src/fs/devfs.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/fs/devfs.rs)
- **Input event types**: [`kernel/kernel/src/syscall/input.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/kernel/src/syscall/input.rs)

## Example Usage

### Keyboard Input

```c
// Open raw keyboard device
int fd = open("/dev/kbd", O_RDONLY);

struct key_event {
    uint16_t scancode;
    uint8_t pressed;  // 1 = pressed, 0 = released
};

while (1) {
    struct key_event ev;
    read(fd, &ev, sizeof(ev));
    printf("Key %02X %s\n", ev.scancode,
           ev.pressed ? "pressed" : "released");
}
```

### Mouse Input

```c
int fd = open("/dev/mouse", O_RDONLY);

struct mouse_event {
    int16_t dx, dy;
    uint8_t buttons;  // Bit 0=left, 1=right, 2=middle
};

while (1) {
    struct mouse_event ev;
    read(fd, &ev, sizeof(ev));
    printf("Mouse delta: (%d, %d), buttons: %02X\n",
           ev.dx, ev.dy, ev.buttons);
}
```

## Interaction with TTY

When a user is in a TTY shell, both the TTY line discipline **and** the raw device nodes receive the same events. This allows:

- **Interactive shells** to process keystrokes normally
- **Background programs** to optionally read raw events
- **Exclusive access** (e.g., games) to temporarily drain raw devices

There is no built-in locking or mutual exclusion. Applications using raw devices are responsible for not interfering with running TTYs.

## Future Enhancements

- **Event queuing**: Bounded queue with overflow handling
- **Joystick support**: `/dev/js0` for game controllers
- **Touchscreen support**: `/dev/touch0` for touch input
- **Event filtering**: ioctl to configure which events are reported

## References

- [PS/2 Keyboard specification](https://wiki.osdev.org/PS/2_Keyboard)
- [PS/2 Mouse specification](https://wiki.osdev.org/PS/2_Mouse)
- [Linux input subsystem](https://www.kernel.org/doc/html/latest/input/)
