# Phase 15: Compositor & 2D Graphics

## Goal

Build a userspace software-rendering 2D graphics library and a simple window compositor. After this phase, multiple userspace programs can draw into windows, a compositor manages the window stack and input dispatch, and the user sees a graphical desktop with mouse cursor, window dragging, and focus management.

## Overview

This phase is entirely userspace code. The kernel already provides everything needed:

- `/dev/fb0` — mmap-able framebuffer (Phase 13)
- `/dev/mouse` — mouse events (Phase 13)
- `/dev/kbd` — raw keyboard events (Phase 13)
- VirtIO GPU flush/cursor (Phase 14, optional — Bochs VGA works too)

The compositor is a privileged userspace process that owns the framebuffer and dispatches input to client windows.

## Key Components

### 1. 2D Graphics Library (`lepton-gfx`)

A minimal software-rendering library for userspace programs:

```rust
/// A 32-bit BGRX pixel buffer.
pub struct Surface {
    data: &mut [u32],
    width: u32,
    height: u32,
    pitch: u32,  // In pixels
}

impl Surface {
    /// Fill a rectangle with a solid color.
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32);

    /// Draw a horizontal line.
    pub fn hline(&mut self, x: u32, y: u32, w: u32, color: u32);

    /// Draw a vertical line.
    pub fn vline(&mut self, x: u32, y: u32, h: u32, color: u32);

    /// Draw a 1px rectangle outline.
    pub fn draw_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32);

    /// Blit a source surface onto this surface at (dx, dy).
    pub fn blit(&mut self, src: &Surface, dx: u32, dy: u32);

    /// Blit with a source rectangle (sub-region copy).
    pub fn blit_rect(&mut self, src: &Surface, src_rect: Rect, dx: u32, dy: u32);

    /// Draw a character using a bitmap font.
    pub fn draw_char(&mut self, x: u32, y: u32, ch: char, color: u32, font: &BitmapFont);

    /// Draw a string using a bitmap font.
    pub fn draw_text(&mut self, x: u32, y: u32, text: &str, color: u32, font: &BitmapFont);
}
```

The library includes:
- **Pixel operations**: fill, blit, line, rect
- **Bitmap font rendering**: 8x16 built-in font for text display
- **Color utilities**: RGB construction, alpha blending (optional)

### 2. Window Compositor (`lepton-compositor`)

The compositor is a userspace process that:

1. Opens `/dev/fb0` and mmaps the framebuffer as its back buffer.
2. Opens `/dev/mouse` and `/dev/kbd` for input.
3. Manages a stack of client windows.
4. Composites all visible windows into the back buffer each frame.
5. Flushes the buffer to the display.

#### Window Stack

```rust
pub struct Compositor {
    framebuffer: Surface,       // mmap'd /dev/fb0
    back_buffer: Surface,       // Heap-allocated compositing buffer
    windows: Vec<Window>,       // Front-to-back order (index 0 = topmost)
    focused: Option<usize>,     // Index of focused window
    cursor_x: i32,
    cursor_y: i32,
    mouse_fd: usize,
    kbd_fd: usize,
}

pub struct Window {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub surface: Surface,       // Client-rendered content
    pub title: String,
    pub visible: bool,
    pub dragging: bool,
}
```

#### Compositing Loop

```rust
fn composite_loop(comp: &mut Compositor) {
    loop {
        // 1. Process input events
        while let Some(event) = read_mouse_event(comp.mouse_fd) {
            comp.cursor_x = (comp.cursor_x + event.dx as i32).clamp(0, fb_width);
            comp.cursor_y = (comp.cursor_y - event.dy as i32).clamp(0, fb_height);

            if event.buttons & BUTTON_LEFT != 0 {
                handle_click(comp, comp.cursor_x, comp.cursor_y);
            }
        }

        while let Some(event) = read_key_event(comp.kbd_fd) {
            if let Some(focused) = comp.focused {
                dispatch_key_to_window(&mut comp.windows[focused], event);
            }
        }

        // 2. Composite: draw background, then windows back-to-front
        comp.back_buffer.fill_rect(0, 0, fb_width, fb_height, DESKTOP_COLOR);
        for window in comp.windows.iter().rev() {
            if window.visible {
                draw_window_frame(&mut comp.back_buffer, window);
                comp.back_buffer.blit(&window.surface, window.x, window.y);
            }
        }

        // 3. Draw cursor
        draw_cursor(&mut comp.back_buffer, comp.cursor_x, comp.cursor_y);

        // 4. Flip: copy back buffer to framebuffer
        comp.framebuffer.blit(&comp.back_buffer, 0, 0);
    }
}
```

#### Window Dragging

Click-and-drag on a window's title bar moves the window:

1. On mouse-down in a title bar region, set `window.dragging = true`.
2. On mouse-move while dragging, update `window.x` and `window.y` by the mouse delta.
3. On mouse-up, set `window.dragging = false`.

#### Focus Management

- Click on a window raises it to the top of the stack and sets focus.
- Keyboard events are dispatched to the focused window.
- Alt+Tab (or similar) cycles focus through the window stack.

### 3. Client-Compositor Communication

Clients communicate with the compositor through a simple IPC mechanism:

- **Shared memory**: Each client allocates a surface buffer. The compositor maps it (or the client writes to a pipe). For the initial implementation, clients can use a simple pipe-based protocol.
- **Protocol messages**: Create window, resize, close, key event, mouse event.

A minimal initial approach: each client is a child process of the compositor, with shared memory regions for window surfaces and pipes for events.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `userspace/lepton-gfx/src/lib.rs` | **New crate:** 2D graphics library (Surface, drawing primitives, font) |
| `userspace/lepton-gfx/src/surface.rs` | Surface struct and pixel operations |
| `userspace/lepton-gfx/src/font.rs` | Bitmap font data and text rendering |
| `userspace/compositor/src/main.rs` | **New crate:** Window compositor |
| `userspace/compositor/src/window.rs` | Window struct and management |
| `userspace/compositor/src/input.rs` | Mouse and keyboard event processing |
| `userspace/compositor/src/composite.rs` | Compositing and rendering loop |

## Frame vs Service

All code in this phase is userspace. No kernel modifications are needed.

| Component | Layer | Reason |
|-----------|-------|--------|
| `lepton-gfx` | Userspace library | Pure software rendering |
| Compositor | Userspace process | Window management, input dispatch |
| Bitmap font | Userspace data | Static font data embedded in binary |
| Client IPC | Userspace | Pipes or shared memory via existing syscalls |

## Dependencies

- **Phase 13**: Input & display infrastructure (devfs device nodes, sys_mmap, sys_ioctl).
- **Phase 11**: IPC (pipes for client-compositor communication).
- **Phase 14**: VirtIO GPU (optional — hardware cursor improves UX, but Bochs VGA works).

## Milestone

```
[compositor] mmap'd framebuffer: 1280x720x32bpp
[compositor] opened /dev/mouse, /dev/kbd
[compositor] desktop ready

[client:terminal] created window "Terminal" (640x480)
[client:clock] created window "Clock" (200x100)

[compositor] compositing 2 windows + cursor
[compositor] window "Terminal" focused
[compositor] dragging "Clock" to (400, 200)
```

A graphical desktop with two windows, a mouse cursor, and the ability to drag and focus windows.
