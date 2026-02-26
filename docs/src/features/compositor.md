# Userspace Compositor & Display Protocol

**Status: Completed** (implemented in commit 4c99506)

Hadron provides a userspace compositor that manages window rendering and display protocol for graphical applications. The compositor runs as a privileged userspace service, managing a window stack and coordinating display output through a custom IPC protocol based on channels and shared memory.

Source: [`userspace/lepton-compositor/`](https://github.com/anomalyco/hadron/blob/main/userspace/lepton-compositor/), [`userspace/lepton-display-protocol/`](https://github.com/anomalyco/hadron/blob/main/userspace/lepton-display-protocol/), [`userspace/lepton-display-client/`](https://github.com/anomalyco/hadron/blob/main/userspace/lepton-display-client/)

## Architecture

The display system operates as a service-client model:

```
┌─────────────────────────────────────────────────┐
│ Userspace Compositor (lepton-compositor)        │
│  - Window stack management                      │
│  - Framebuffer rendering (back-buffer)          │
│  - Input event dispatch                         │
└─────────────────────────────────────────────────┘
         ▲                           ▼
    Display Protocol (channels + shared memory)
         ▲                           ▼
┌─────────────────────────────────────────────────┐
│ Application Clients                             │
│  - lepton-display-client library                │
│  - Render to shared memory buffers              │
│  - Receive input events                         │
└─────────────────────────────────────────────────┘
```

## Display Protocol

The display protocol defines a client-server IPC interface using channels for request/response messages and shared memory for zero-copy framebuffer sharing.

### Request Types

| Request | Purpose |
|---------|---------|
| `CREATE_WINDOW` | Create a new window with specified dimensions |
| `DESTROY_WINDOW` | Destroy a window |
| `SHOW_WINDOW` | Make a window visible on screen |
| `HIDE_WINDOW` | Hide a window |
| `MOVE_WINDOW` | Reposition a window |
| `RESIZE_WINDOW` | Change a window's dimensions |
| `FLUSH` | Request compositor to repaint the screen |

### Response Types

| Response | Carries |
|----------|---------|
| `OK` | Acknowledgment (with optional handle) |
| `ERROR` | Error code and description |
| `EVENT` | Input event (keyboard, mouse) |

## Window Management

The compositor maintains a **window stack** -- a Z-ordered list of windows with occlusion tracking.

### Window Structure

Each window contains:

- **Metadata** -- ID, position (x, y), size (width, height), visibility
- **Surface buffer** -- Shared memory region for framebuffer/back-buffer
- **Owner PID** -- The process that created the window

### Rendering Pipeline

1. **Client renders** -- Writes pixels to its shared memory surface buffer
2. **Client requests FLUSH** -- Sends a flush request via the channel
3. **Compositor receives FLUSH** -- Marks the window as dirty
4. **Compositor composites** -- On the next vsync or timer tick:
   - Iterate window stack from back to front
   - For each visible window, copy its surface to the physical framebuffer
   - Handle occlusion (skip pixels covered by higher windows)
   - Update changed regions only (dirty rectangle optimization)
5. **Compositor presents** -- Update the physical framebuffer (via `/dev/fb0`)

### Dirty Rectangle Tracking

To optimize screen updates, the compositor tracks **dirty rectangles** -- regions of the screen that have changed since the last composite:

- **Per-window dirty rect** -- The region the client dirtied in its surface
- **Global dirty rect** -- The union of all affected screen regions
- **Incremental update** -- Only the dirty region is updated on the physical framebuffer

## Input Event Dispatch

The compositor receives input events from the kernel (via `/dev/console` or `/dev/mouse`) and dispatches them to the appropriate window.

### Event Types

| Event | Data |
|-------|------|
| `KEY_DOWN` | Key code, modifiers (Shift, Ctrl, Alt) |
| `KEY_UP` | Key code, modifiers |
| `MOUSE_MOVE` | X, Y, buttons held |
| `MOUSE_DOWN` | Button (left/middle/right), X, Y |
| `MOUSE_UP` | Button, X, Y |

### Focus Management

The compositor tracks which window has **keyboard focus** and **mouse focus**:

- **Keyboard focus** -- Receives keyboard events
- **Mouse focus** -- Window under the mouse cursor

Focus can be changed via:

- **Mouse click** -- Clicking on a window gives it focus
- **Alt+Tab** -- Cycle focus through windows

## Shared Memory Integration

The compositor and clients use IPC channels for command/response and **shared memory** for zero-copy buffer sharing.

### Buffer Sharing

1. **Client creates window** via `CREATE_WINDOW` request
2. **Compositor allocates shared memory** region for the window's surface
3. **Compositor returns SHM ID** and virtual address in the response
4. **Client maps SHM region** into its address space at the provided address
5. **Client renders directly** into the shared memory
6. **Client requests FLUSH** to signal the compositor to composite

This eliminates the need to copy pixel data between processes; the compositor reads pixels directly from the shared memory.

## Framebuffer Device Integration

The compositor integrates with the kernel's framebuffer device (`/dev/fb0`):

- **MMAP framebuffer** -- The compositor memory-maps `/dev/fb0` to get direct access to the physical framebuffer
- **Scanline-buffered rendering** -- Uses a scanline-sized temporary buffer to optimize cache locality during compositing
- **IOCTL queries** -- Uses `ioctl(TIOCGFB)` to query framebuffer properties (width, height, stride, pixel format)

## Client Library (lepton-display-client)

A userspace library simplifies client-side integration:

```rust
use lepton_display_client::{DisplayClient, Window};

fn main() {
    let mut client = DisplayClient::connect().unwrap();
    let mut window = client.create_window(800, 600).unwrap();
    
    // Render to the window's buffer
    window.fill_rect(0, 0, 800, 600, 0xFF0000); // Red
    
    // Request compositor to refresh
    client.flush().unwrap();
    
    // Wait for input events
    while let Some(event) = client.next_event() {
        match event {
            Event::KeyDown(key) => println!("Key: {}", key),
            Event::MouseMove(x, y) => println!("Mouse: ({}, {})", x, y),
            _ => {}
        }
    }
}
```

## Implementation Status

Compositor process and window stack
Display protocol definition (request/response messages)
Shared memory surface buffers
Framebuffer rendering (direct to `/dev/fb0`)
Scanline-buffered rendering optimization
Window creation, destruction, and positioning
Input event reception and dispatch
Basic occlusion/Z-ordering
Display client library
Dirty rectangle optimization
Vsync synchronization
Per-window clipping
Alpha blending and transparency

## Files to Modify

- `userspace/lepton-compositor/src/main.rs` -- Compositor main loop and window management
- `userspace/lepton-compositor/src/render.rs` -- Rendering and compositing logic
- `userspace/lepton-display-protocol/src/lib.rs` -- Display protocol definition
- `userspace/lepton-display-client/src/lib.rs` -- Client library for applications
- `kernel/kernel/src/fs/devfs.rs` -- Framebuffer device integration

## References

- **IPC Channels & Shared Memory**: [IPC Channels & Shared Memory](../features/ipc-channels.md)
- **Display Infrastructure**: [Display Infrastructure](../features/display-infrastructure.md)
- **Task Execution**: [Task Execution & Scheduling](../architecture/task-execution.md)

