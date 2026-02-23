# Phase 13: Input & Display Infrastructure

## Goal

Expose PS/2 mouse, raw keyboard events, and the framebuffer to userspace via devfs device nodes. Add `sys_mmap` support for device memory and `sys_ioctl` for framebuffer queries. After this phase, userspace programs can read mouse movement, capture raw key events independently of the TTY, and draw directly to the display by mapping the framebuffer into their address space.

## Prerequisites

The following hardware drivers already exist from Phase 10:

- **`AsyncMouse`** — PS/2 mouse driver (IRQ 12), produces dx/dy/buttons events.
- **`AsyncKeyboard`** — PS/2 keyboard driver (IRQ 1), produces scancodes.
- **Bochs VGA** — 1280x720x32bpp linear framebuffer via PCI BAR0.

This phase bridges these drivers to userspace through the VFS.

## Key Design

### Device Nodes

Three new device nodes are added to devfs:

| Device | Path | Description |
|--------|------|-------------|
| Mouse | `/dev/mouse` | Reads produce `MouseEvent` structs (dx, dy, buttons) |
| Keyboard | `/dev/kbd` | Reads produce raw `KeyEvent` structs (scancode, pressed/released) |
| Framebuffer | `/dev/fb0` | Mmap-able linear framebuffer; ioctl for display info |

#### Mouse Device

```rust
pub struct MouseEvent {
    pub dx: i16,
    pub dy: i16,
    pub buttons: u8,  // Bit 0 = left, bit 1 = right, bit 2 = middle
}

/// DevFS inode backed by AsyncMouse.
/// read() returns MouseEvent structs. Blocks (awaits) if no events pending.
struct MouseDeviceInode {
    mouse: Arc<AsyncMouse>,
}

impl Inode for MouseDeviceInode {
    fn read<'a>(&'a self, _offset: usize, buf: &'a mut [u8])
        -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>>
    {
        Box::pin(async move {
            let event = self.mouse.next_event().await;
            let bytes = event.as_bytes();
            let len = buf.len().min(bytes.len());
            buf[..len].copy_from_slice(&bytes[..len]);
            Ok(len)
        })
    }
}
```

#### Keyboard Device

```rust
pub struct KeyEvent {
    pub scancode: u16,
    pub pressed: bool,
}

/// DevFS inode backed by AsyncKeyboard.
/// Separate from the TTY line discipline — provides raw scancodes.
struct KbdDeviceInode {
    keyboard: Arc<AsyncKeyboard>,
}
```

The raw keyboard device provides scancodes without TTY processing (no line editing, no echo, no Ctrl+C interpretation). This is essential for games, compositors, and any program that needs direct key input.

#### Framebuffer Device

```rust
/// DevFS inode for /dev/fb0.
/// Supports mmap to map the linear framebuffer into userspace.
/// Supports ioctl to query display parameters.
struct FbDeviceInode {
    fb: Arc<Framebuffer>,
}

#[repr(C)]
pub struct FbInfo {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,      // Bytes per scanline
    pub bpp: u32,        // Bits per pixel (32)
    pub format: u32,     // Pixel format (BGRX8888)
    pub size: u64,       // Total framebuffer size in bytes
}
```

### sys_mmap for Device Memory

The framebuffer's physical memory (Bochs VGA PCI BAR0) is mapped into userspace via `sys_mmap`:

```rust
pub fn sys_mmap(
    addr: usize,       // Hint or 0 for kernel-chosen
    length: usize,
    prot: u32,         // PROT_READ | PROT_WRITE
    flags: u32,        // MAP_SHARED
    fd: usize,         // File descriptor for /dev/fb0
    offset: u64,       // Offset into device memory
) -> Result<usize, SyscallError> {
    let inode = get_inode_for_fd(fd)?;
    // If the inode supports mmap, map the backing physical pages
    // into the process address space with the requested permissions.
    inode.mmap(process_address_space, addr, length, prot, offset)
}
```

For `/dev/fb0`, the mmap implementation maps the framebuffer's physical pages (from the PCI BAR) directly into the user address space with write-combining caching. This allows userspace to draw pixels by writing to the mapped region.

### sys_ioctl

```rust
pub fn sys_ioctl(
    fd: usize,
    request: u64,
    arg: usize,
) -> Result<usize, SyscallError> {
    let inode = get_inode_for_fd(fd)?;
    inode.ioctl(request, arg)
}
```

For `/dev/fb0`:
- `FBIOGET_INFO` — copies `FbInfo` to the userspace pointer in `arg`.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/fs/devfs.rs` | Add mouse, kbd, and fb0 device node registration |
| `hadron-kernel/src/drivers/input_dev.rs` | **New:** devfs ↔ AsyncMouse/AsyncKeyboard bridge inodes |
| `hadron-kernel/src/driver_api/framebuffer.rs` | Add mmap support and FbInfo struct |
| `hadron-kernel/src/syscall/mmap.rs` | **New:** sys_mmap implementation |
| `hadron-kernel/src/syscall/ioctl.rs` | **New:** sys_ioctl implementation |
| `hadron-kernel/src/fs/mod.rs` | Add `mmap()` and `ioctl()` methods to Inode trait |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| MouseDeviceInode | Service | Bridges async driver to VFS inode interface |
| KbdDeviceInode | Service | Bridges async driver to VFS inode interface |
| FbDeviceInode | Service | VFS inode with mmap/ioctl support |
| sys_mmap (page table mapping) | Frame | Maps physical pages into user address space |
| sys_ioctl dispatch | Service | Routes ioctl to inode implementation |
| FbInfo struct | Service | Plain data structure |

## Dependencies

- **Phase 8**: VFS and devfs (device node registration, Inode trait).
- **Phase 9**: Userspace (user address spaces for mmap).
- **Phase 10**: Device drivers (AsyncMouse, AsyncKeyboard, Bochs VGA framebuffer).

## Milestone

```
devfs: registered /dev/mouse (PS/2 mouse)
devfs: registered /dev/kbd (raw keyboard)
devfs: registered /dev/fb0 (framebuffer 1280x720x32bpp)

[userspace] mmap /dev/fb0 -> 0x4000_0000 (3686400 bytes)
[userspace] ioctl FBIOGET_INFO: 1280x720, pitch=5120, bpp=32
[userspace] drawing red rectangle at (100, 100)...
[userspace] reading /dev/mouse: dx=5, dy=-3, buttons=0x00
```
