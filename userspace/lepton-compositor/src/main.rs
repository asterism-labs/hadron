//! Minimal userspace compositor for Hadron.
//!
//! Owns `/dev/fb0`, manages client surfaces via channels + shared memory,
//! composites all surfaces into a back buffer, and flips to the framebuffer.
//! Keyboard input is forwarded to the focused client.
//!
//! Clients are spawned with fd 3 = channel endpoint, fd 4 = shared memory fd.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use lepton_display_protocol::{self as proto, MESSAGE_SIZE, OP_COMMIT};
use lepton_gfx::Surface;
use lepton_syslib::hadron_syscall::{
    self as hsc, ECHO, FBIOBLANK, FBIODIRTY, FBIOGET_INFO, FbDirtyRect, FbInfo, ICANON, TCGETS,
    TCSETS, Termios,
};
use lepton_syslib::{io, println, sys};

/// Bytes per pixel (32-bit color).
const BPP: usize = 4;

/// Frame interval in milliseconds (~60 fps).
const FRAME_MS: u64 = 16;

/// Background color for uncovered framebuffer regions.
const BG_COLOR: u32 = 0x0020_2020;

/// Border color for the focused surface.
const FOCUS_BORDER: u32 = 0x00FF_FFFF; // white

/// Height of the server-side titlebar in pixels.
const TITLEBAR_HEIGHT: u32 = 20;

/// Width of the close button region in the titlebar.
const CLOSE_BTN_WIDTH: u32 = 20;

/// Titlebar background color for the focused window.
const TITLEBAR_FOCUSED: u32 = 0x0040_6080;

/// Titlebar background color for unfocused windows.
const TITLEBAR_UNFOCUSED: u32 = 0x0040_4040;

/// Titlebar text color.
const TITLEBAR_TEXT: u32 = 0x00FF_FFFF;

/// Close button text color.
const CLOSE_BTN_COLOR: u32 = 0x00FF_6060;

// ── Client state ─────────────────────────────────────────────────────

/// Per-client state tracked by the compositor.
struct ClientState {
    /// Child process PID (used by `close_client` to SIGKILL the process).
    pid: u32,
    /// Compositor's channel endpoint fd (reads Commit, writes Configure/Key).
    channel_fd: usize,
    /// Compositor's mapping of the client's shared pixel buffer.
    shm_ptr: *mut u8,
    /// Size of the shared-memory region in bytes.
    shm_size: usize,
    /// Shared-memory fd (kept open so the mapping stays valid).
    shm_fd: usize,
    /// Surface placement and dimensions.
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// Whether the client committed at least once since last composite.
    dirty: bool,
    /// Window title (extracted from the executable path).
    title: [u8; 32],
    /// Number of valid bytes in `title`.
    title_len: usize,
}

// ── Drag state ──────────────────────────────────────────────────────

/// Tracks an active titlebar drag operation.
struct DragState {
    /// Index of the client being dragged.
    client_idx: usize,
    /// Cursor offset from the window's top-left corner at drag start.
    offset_x: i32,
    /// Cursor offset from the window's top-left corner at drag start.
    offset_y: i32,
}

// ── Cursor sprite ───────────────────────────────────────────────────

/// 12x19 arrow cursor bitmap: 0 = transparent, 1 = white outline, 2 = black fill.
const CURSOR_W: u32 = 12;
const CURSOR_H: u32 = 19;
#[rustfmt::skip]
const CURSOR_BITMAP: [u8; (CURSOR_W * CURSOR_H) as usize] = [
    1,0,0,0,0,0,0,0,0,0,0,0,
    1,1,0,0,0,0,0,0,0,0,0,0,
    1,2,1,0,0,0,0,0,0,0,0,0,
    1,2,2,1,0,0,0,0,0,0,0,0,
    1,2,2,2,1,0,0,0,0,0,0,0,
    1,2,2,2,2,1,0,0,0,0,0,0,
    1,2,2,2,2,2,1,0,0,0,0,0,
    1,2,2,2,2,2,2,1,0,0,0,0,
    1,2,2,2,2,2,2,2,1,0,0,0,
    1,2,2,2,2,2,2,2,2,1,0,0,
    1,2,2,2,2,2,2,2,2,2,1,0,
    1,2,2,2,2,2,1,1,1,1,1,1,
    1,2,2,2,2,2,1,0,0,0,0,0,
    1,2,2,1,2,2,1,0,0,0,0,0,
    1,2,1,0,1,2,2,1,0,0,0,0,
    1,1,0,0,1,2,2,1,0,0,0,0,
    1,0,0,0,0,1,2,2,1,0,0,0,
    0,0,0,0,0,1,2,2,1,0,0,0,
    0,0,0,0,0,0,1,1,0,0,0,0,
];

// ── Compositor ───────────────────────────────────────────────────────

/// Top-level compositor state.
struct Compositor {
    /// Framebuffer pointer (mmap of `/dev/fb0`).
    fb_ptr: *mut u8,
    /// Framebuffer dimensions.
    fb_width: u32,
    fb_height: u32,
    /// Framebuffer stride in pixels (pitch / 4).
    fb_stride: u32,
    /// Total framebuffer size in bytes.
    fb_size: usize,
    /// Framebuffer file descriptor.
    fb_fd: usize,
    /// Back buffer (anonymous mmap, same layout as fb).
    back_ptr: *mut u8,
    /// Connected clients (Z-order: last = topmost).
    clients: Vec<ClientState>,
    /// Index of the focused client, if any.
    focused: Option<usize>,
    /// Whether the scene needs re-compositing.
    needs_composite: bool,
    /// File descriptor for `/dev/mouse`, if opened successfully.
    mouse_fd: Option<usize>,
    /// Absolute cursor X position.
    cursor_x: i32,
    /// Absolute cursor Y position.
    cursor_y: i32,
    /// Current mouse button state.
    buttons: u8,
    /// Previous frame's button state (for click detection).
    prev_buttons: u8,
    /// Active titlebar drag operation.
    dragging: Option<DragState>,
}

impl Compositor {
    /// Open the framebuffer and allocate the back buffer.
    fn init() -> Option<Self> {
        let fd = io::open("/dev/fb0", 0);
        if fd < 0 {
            println!("compositor: failed to open /dev/fb0");
            return None;
        }
        let fb_fd = fd as usize;

        let mut info = FbInfo {
            width: 0,
            height: 0,
            pitch: 0,
            bpp: 0,
            pixel_format: 0,
        };
        if io::ioctl(
            fb_fd,
            FBIOGET_INFO as usize,
            &mut info as *mut FbInfo as usize,
        ) < 0
        {
            println!("compositor: ioctl FBIOGET_INFO failed");
            io::close(fb_fd);
            return None;
        }

        // Disable kernel fbcon output — the compositor owns the framebuffer now.
        io::ioctl(fb_fd, FBIOBLANK as usize, 1);

        let fb_size = info.pitch as usize * info.height as usize;
        let fb_ptr = sys::mem_map_device(fb_fd, fb_size)?;

        // Allocate back buffer via anonymous mmap.
        let back_ptr = sys::mem_map(fb_size)?;

        // Try to open /dev/mouse for cursor input.
        let mouse_fd = {
            let fd = io::open("/dev/mouse", 1); // READ
            if fd >= 0 { Some(fd as usize) } else { None }
        };

        Some(Compositor {
            fb_ptr,
            fb_width: info.width,
            fb_height: info.height,
            fb_stride: info.pitch / 4,
            fb_size,
            fb_fd,
            back_ptr,
            clients: Vec::new(),
            focused: None,
            needs_composite: true,
            mouse_fd,
            cursor_x: (info.width / 2) as i32,
            cursor_y: (info.height / 2) as i32,
            buttons: 0,
            prev_buttons: 0,
            dragging: None,
        })
    }

    /// Spawn a client application at the given window position and size.
    fn spawn_client(&mut self, path: &str, x: u32, y: u32, w: u32, h: u32) {
        // Create channel pair.
        let (ch_compositor, ch_client) = match sys::channel_create() {
            Ok(pair) => pair,
            Err(e) => {
                println!("compositor: channel_create failed: {}", e);
                return;
            }
        };

        // Create shared memory for the pixel buffer.
        let shm_size = w as usize * h as usize * BPP;
        let shm_fd = match sys::mem_create_shared(shm_size) {
            Ok(fd) => fd,
            Err(e) => {
                println!("compositor: mem_create_shared failed: {}", e);
                sys::close(ch_compositor);
                sys::close(ch_client);
                return;
            }
        };

        // Compositor maps the shm to read client pixels.
        let shm_ptr = match sys::mem_map_shared(shm_fd, shm_size) {
            Some(p) => p,
            None => {
                println!("compositor: mem_map_shared failed");
                sys::close(ch_compositor);
                sys::close(ch_client);
                sys::close(shm_fd);
                return;
            }
        };

        // Spawn the child with fd 3 = channel, fd 4 = shm.
        let pid = sys::spawn_with_fds(
            path,
            &[path],
            &[
                (0, 0), // stdin
                (1, 1), // stdout
                (2, 2), // stderr
                (3, ch_client as u32),
                (4, shm_fd as u32),
            ],
        );

        // Close the child's channel end in the compositor.
        sys::close(ch_client);

        if pid < 0 {
            println!("compositor: spawn {} failed: {}", path, pid);
            sys::close(ch_compositor);
            sys::mem_unmap(shm_ptr, shm_size);
            sys::close(shm_fd);
            return;
        }

        // Extract filename from path as the window title.
        let mut title = [0u8; 32];
        let mut title_len = 0;
        let name = path.rsplit('/').next().unwrap_or(path);
        for (i, &b) in name.as_bytes().iter().enumerate() {
            if i >= title.len() {
                break;
            }
            title[i] = b;
            title_len = i + 1;
        }

        let client_idx = self.clients.len();
        self.clients.push(ClientState {
            pid: pid as u32,
            channel_fd: ch_compositor,
            shm_ptr,
            shm_size,
            shm_fd,
            x,
            y,
            width: w,
            height: h,
            dirty: false,
            title,
            title_len,
        });

        // Focus the new client.
        self.focused = Some(client_idx);
        self.needs_composite = true;

        // Send Configure message.
        let cfg = proto::configure(0, w, h);
        let mut buf = [0u8; MESSAGE_SIZE];
        // SAFETY: Configure is a 64-byte repr(C) message.
        unsafe { proto::encode_msg(&cfg, &mut buf) };
        let _ = sys::channel_send(ch_compositor, &buf);

        // Send FocusGained to the new client.
        let fg = proto::focus_gained(0);
        // SAFETY: FocusGained is a 64-byte repr(C) message.
        unsafe { proto::encode_msg(&fg, &mut buf) };
        let _ = sys::channel_send(ch_compositor, &buf);
    }

    /// Poll all clients for Commit messages.
    fn poll_clients(&mut self) {
        for client in &mut self.clients {
            if sys::poll_fd_read(client.channel_fd) {
                let mut buf = [0u8; MESSAGE_SIZE];
                if let Ok(n) = sys::channel_recv(client.channel_fd, &mut buf) {
                    if n >= MESSAGE_SIZE && proto::peek_opcode(&buf) == OP_COMMIT {
                        client.dirty = true;
                    }
                }
            }
        }

        if self.clients.iter().any(|c| c.dirty) {
            self.needs_composite = true;
        }
    }

    /// Handle keyboard input from stdin.
    fn poll_keyboard(&mut self) {
        while sys::poll_stdin() {
            let mut byte = [0u8; 1];
            let n = io::read(0, &mut byte);
            if n <= 0 {
                break;
            }

            // ESC (0x1B) followed by TAB (0x09) = Alt+Tab / focus cycle.
            if byte[0] == 0x1B {
                // Check if a second byte follows immediately.
                if sys::poll_stdin() {
                    let mut next = [0u8; 1];
                    let n2 = io::read(0, &mut next);
                    if n2 > 0 && next[0] == 0x09 {
                        self.cycle_focus();
                        continue;
                    }
                    // Not TAB — forward ESC then the next byte.
                    self.forward_key(0x1B);
                    self.forward_key(next[0]);
                    continue;
                }
                // Lone ESC — forward it.
            }

            self.forward_key(byte[0]);
        }
    }

    /// Forward a keyboard byte to the focused client.
    fn forward_key(&self, character: u8) {
        let Some(idx) = self.focused else { return };
        let client = &self.clients[idx];

        let msg = proto::keyboard_input(0, u32::from(character), true, character);
        let mut buf = [0u8; MESSAGE_SIZE];
        // SAFETY: KeyboardInput is a 64-byte repr(C) message.
        unsafe { proto::encode_msg(&msg, &mut buf) };
        let _ = sys::channel_send(client.channel_fd, &buf);
    }

    /// Cycle keyboard focus to the next client.
    fn cycle_focus(&mut self) {
        if self.clients.is_empty() {
            return;
        }

        let old = self.focused;
        let new_idx = match old {
            Some(i) => (i + 1) % self.clients.len(),
            None => 0,
        };

        // Send FocusLost to old client.
        if let Some(old_idx) = old {
            let msg = proto::focus_lost(0);
            let mut buf = [0u8; MESSAGE_SIZE];
            // SAFETY: FocusLost is a 64-byte repr(C) message.
            unsafe { proto::encode_msg(&msg, &mut buf) };
            let _ = sys::channel_send(self.clients[old_idx].channel_fd, &buf);
        }

        // Send FocusGained to new client.
        let msg = proto::focus_gained(0);
        let mut buf = [0u8; MESSAGE_SIZE];
        // SAFETY: FocusGained is a 64-byte repr(C) message.
        unsafe { proto::encode_msg(&msg, &mut buf) };
        let _ = sys::channel_send(self.clients[new_idx].channel_fd, &buf);

        // Move focused surface to top of Z-order.
        let client = self.clients.remove(new_idx);
        self.clients.push(client);
        self.focused = Some(self.clients.len() - 1);

        self.needs_composite = true;
    }

    /// Focus a specific client by index, sending focus lost/gained messages.
    fn focus_client(&mut self, idx: usize) {
        if self.focused == Some(idx) {
            return;
        }

        // Send FocusLost to old.
        if let Some(old_idx) = self.focused {
            let msg = proto::focus_lost(0);
            let mut buf = [0u8; MESSAGE_SIZE];
            // SAFETY: FocusLost is a 64-byte repr(C) message.
            unsafe { proto::encode_msg(&msg, &mut buf) };
            let _ = sys::channel_send(self.clients[old_idx].channel_fd, &buf);
        }

        // Send FocusGained to new.
        let msg = proto::focus_gained(0);
        let mut buf = [0u8; MESSAGE_SIZE];
        // SAFETY: FocusGained is a 64-byte repr(C) message.
        unsafe { proto::encode_msg(&msg, &mut buf) };
        let _ = sys::channel_send(self.clients[idx].channel_fd, &buf);

        // Move to top of Z-order.
        let client = self.clients.remove(idx);
        self.clients.push(client);
        self.focused = Some(self.clients.len() - 1);

        self.needs_composite = true;
    }

    /// Close a client window: kill the process and clean up resources.
    fn close_client(&mut self, idx: usize) {
        let client = self.clients.remove(idx);
        sys::kill(client.pid, 9); // SIGKILL
        sys::close(client.channel_fd);
        sys::mem_unmap(client.shm_ptr, client.shm_size);
        sys::close(client.shm_fd);

        // Update focused index after removal.
        if let Some(foc) = self.focused {
            if foc == idx {
                // Focused window was closed — focus the new topmost.
                self.focused = if self.clients.is_empty() {
                    None
                } else {
                    let new_top = self.clients.len() - 1;
                    let msg = proto::focus_gained(0);
                    let mut buf = [0u8; MESSAGE_SIZE];
                    // SAFETY: FocusGained is a 64-byte repr(C) message.
                    unsafe { proto::encode_msg(&msg, &mut buf) };
                    let _ = sys::channel_send(self.clients[new_top].channel_fd, &buf);
                    Some(new_top)
                };
            } else if foc > idx {
                self.focused = Some(foc - 1);
            }
        }

        self.needs_composite = true;
    }

    /// Read all pending mouse events from /dev/mouse and process them.
    fn poll_mouse(&mut self) {
        let Some(mouse_fd) = self.mouse_fd else {
            return;
        };

        // Read mouse event packets (8 bytes each).
        let mut raw = [0u8; 128]; // up to 16 events
        while sys::poll_fd_read(mouse_fd) {
            let n = io::read(mouse_fd, &mut raw);
            if n <= 0 {
                break;
            }
            let n = n as usize;
            let packet_size = core::mem::size_of::<hsc::MouseEventPacket>();
            let count = n / packet_size;
            for i in 0..count {
                let offset = i * packet_size;
                let pkt: hsc::MouseEventPacket =
                    // SAFETY: MouseEventPacket is repr(C), 8 bytes, and buffer has that many.
                    unsafe { core::ptr::read_unaligned(raw.as_ptr().add(offset).cast()) };
                // Accumulate movement (negate dy for screen coordinates).
                self.cursor_x += i32::from(pkt.dx);
                self.cursor_y -= i32::from(pkt.dy);
                // Clamp to screen bounds.
                self.cursor_x = self.cursor_x.clamp(0, self.fb_width as i32 - 1);
                self.cursor_y = self.cursor_y.clamp(0, self.fb_height as i32 - 1);
                self.prev_buttons = self.buttons;
                self.buttons = pkt.buttons;
            }
            self.needs_composite = true;
        }

        self.handle_mouse_buttons();
    }

    /// Process mouse button state changes (click, drag, release).
    fn handle_mouse_buttons(&mut self) {
        let left_pressed = self.buttons & 0x01 != 0;
        let left_was_pressed = self.prev_buttons & 0x01 != 0;

        // Left button just pressed.
        if left_pressed && !left_was_pressed {
            let cx = self.cursor_x;
            let cy = self.cursor_y;

            // Hit test windows top-to-bottom (reverse Z-order = highest index first).
            let mut hit = None;
            for i in (0..self.clients.len()).rev() {
                let c = &self.clients[i];
                let total_h = TITLEBAR_HEIGHT + c.height;
                if cx >= c.x as i32
                    && cx < (c.x + c.width) as i32
                    && cy >= c.y as i32
                    && cy < (c.y + total_h) as i32
                {
                    hit = Some(i);
                    break;
                }
            }

            if let Some(idx) = hit {
                let c = &self.clients[idx];
                let in_titlebar = cy < (c.y + TITLEBAR_HEIGHT) as i32;
                let in_close_btn = in_titlebar
                    && c.width >= CLOSE_BTN_WIDTH
                    && cx >= (c.x + c.width - CLOSE_BTN_WIDTH) as i32;

                if in_close_btn {
                    self.close_client(idx);
                } else if in_titlebar {
                    self.focus_client(idx);
                    // After focus_client, the window is at the end of the vec.
                    let new_idx = self.clients.len() - 1;
                    self.dragging = Some(DragState {
                        client_idx: new_idx,
                        offset_x: cx - self.clients[new_idx].x as i32,
                        offset_y: cy - self.clients[new_idx].y as i32,
                    });
                } else {
                    // Click in client area.
                    self.focus_client(idx);
                    let new_idx = self.clients.len() - 1;
                    self.forward_mouse_to_client(new_idx);
                }
            }
        }

        // Left button released.
        if !left_pressed && left_was_pressed {
            self.dragging = None;
        }

        // Update drag position while held.
        if left_pressed {
            if let Some(ref drag) = self.dragging {
                let idx = drag.client_idx;
                let new_x = (self.cursor_x - drag.offset_x).max(0) as u32;
                let new_y = (self.cursor_y - drag.offset_y).max(0) as u32;
                self.clients[idx].x = new_x;
                self.clients[idx].y = new_y;
                self.needs_composite = true;
            }
        }
    }

    /// Forward a mouse event to the given client.
    fn forward_mouse_to_client(&self, idx: usize) {
        let c = &self.clients[idx];
        let rel_x = self.cursor_x - c.x as i32;
        let rel_y = self.cursor_y - (c.y + TITLEBAR_HEIGHT) as i32;
        let msg = proto::mouse_input(
            0,
            rel_x,
            rel_y,
            u32::from(self.buttons),
            u32::from(self.prev_buttons),
        );
        let mut buf = [0u8; MESSAGE_SIZE];
        // SAFETY: MouseInput is a 64-byte repr(C) message.
        unsafe { proto::encode_msg(&msg, &mut buf) };
        let _ = sys::channel_send(c.channel_fd, &buf);
    }

    /// Draw the mouse cursor sprite at the current position.
    fn draw_cursor(&self, back: &mut Surface<'_>) {
        for row in 0..CURSOR_H {
            for col in 0..CURSOR_W {
                let val = CURSOR_BITMAP[(row * CURSOR_W + col) as usize];
                let color = match val {
                    1 => 0x00FF_FFFF, // white outline
                    2 => 0x0010_1010, // near-black fill
                    _ => continue,    // transparent
                };
                back.put_pixel(
                    (self.cursor_x as u32).wrapping_add(col),
                    (self.cursor_y as u32).wrapping_add(row),
                    color,
                );
            }
        }
    }

    /// Composite all client surfaces into the back buffer, then flip.
    fn composite(&mut self) {
        if !self.needs_composite {
            return;
        }

        let pixel_count = (self.fb_stride * self.fb_height) as usize;
        // SAFETY: back_ptr is a valid anonymous mapping of fb_size bytes.
        let back_pixels =
            unsafe { core::slice::from_raw_parts_mut(self.back_ptr as *mut u32, pixel_count) };
        let mut back =
            Surface::from_raw(back_pixels, self.fb_width, self.fb_height, self.fb_stride);

        // Clear background.
        back.fill(BG_COLOR);

        // Blit each client surface (back-to-front) with server-side decorations.
        for (i, client) in self.clients.iter_mut().enumerate() {
            let focused = self.focused == Some(i);
            let x = client.x;
            let y = client.y;
            let w = client.width;

            // 1. Draw titlebar rectangle.
            let tb_color = if focused {
                TITLEBAR_FOCUSED
            } else {
                TITLEBAR_UNFOCUSED
            };
            back.fill_rect(x, y, w, TITLEBAR_HEIGHT, tb_color);

            // 2. Draw title text left-aligned (with 4px left margin, 2px top margin).
            let title_str = core::str::from_utf8(&client.title[..client.title_len]).unwrap_or("?");
            back.draw_str(x + 4, y + 2, title_str, TITLEBAR_TEXT, tb_color);

            // 3. Draw close [X] button right-aligned in titlebar.
            if w >= CLOSE_BTN_WIDTH {
                let close_x = x + w - CLOSE_BTN_WIDTH + 4;
                back.draw_str(close_x, y + 2, "X", CLOSE_BTN_COLOR, tb_color);
            }

            // 4. Blit client surface below the titlebar.
            let surface_y = y + TITLEBAR_HEIGHT;
            let src_pixel_count = client.width as usize * client.height as usize;
            // SAFETY: shm_ptr is a valid mapping of client's pixel buffer.
            let src_pixels = unsafe {
                core::slice::from_raw_parts_mut(client.shm_ptr as *mut u32, src_pixel_count)
            };
            let src = Surface::from_raw(src_pixels, client.width, client.height, client.width);
            back.blit(&src, x as i32, surface_y as i32);

            // 5. Draw 2px border around entire window (titlebar + client area).
            let total_h = TITLEBAR_HEIGHT + client.height;
            let border_color = if focused {
                FOCUS_BORDER
            } else {
                TITLEBAR_UNFOCUSED
            };
            // Top edge (2 lines).
            if y >= 2 {
                back.hline(x.saturating_sub(2), y - 2, w + 4, border_color);
                back.hline(x.saturating_sub(2), y - 1, w + 4, border_color);
            } else if y >= 1 {
                back.hline(x.saturating_sub(2), y - 1, w + 4, border_color);
            }
            // Bottom edge (2 lines).
            back.hline(x.saturating_sub(2), y + total_h, w + 4, border_color);
            back.hline(x.saturating_sub(2), y + total_h + 1, w + 4, border_color);
            // Left edge (2 columns).
            if x >= 2 {
                back.vline(x - 2, y, total_h, border_color);
                back.vline(x - 1, y, total_h, border_color);
            } else if x >= 1 {
                back.vline(x - 1, y, total_h, border_color);
            }
            // Right edge (2 columns).
            back.vline(x + w, y, total_h, border_color);
            back.vline(x + w + 1, y, total_h, border_color);

            client.dirty = false;
        }

        // Draw mouse cursor on top of everything.
        if self.mouse_fd.is_some() {
            self.draw_cursor(&mut back);
        }

        // Flip: copy back buffer to framebuffer.
        // SAFETY: Both pointers are valid mappings of fb_size bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(self.back_ptr, self.fb_ptr, self.fb_size);
        }

        // Notify the framebuffer driver that the entire screen is dirty.
        // For RAM-backed framebuffers (VirtIO GPU) this triggers the host
        // display update; for MMIO-backed ones (Bochs VGA) it's a no-op.
        let dirty = FbDirtyRect {
            x: 0,
            y: 0,
            width: self.fb_width,
            height: self.fb_height,
        };
        io::ioctl(
            self.fb_fd,
            FBIODIRTY as usize,
            &dirty as *const FbDirtyRect as usize,
        );

        self.needs_composite = false;
    }

    /// Clean up resources.
    fn shutdown(&mut self) {
        // Clear framebuffer.
        let pixel_count = (self.fb_stride * self.fb_height) as usize;
        // SAFETY: fb_ptr is still a valid mapping.
        let pixels =
            unsafe { core::slice::from_raw_parts_mut(self.fb_ptr as *mut u32, pixel_count) };
        let mut s = Surface::from_raw(pixels, self.fb_width, self.fb_height, self.fb_stride);
        s.fill(0);

        // Unmap and close client resources.
        for client in &self.clients {
            sys::close(client.channel_fd);
            sys::mem_unmap(client.shm_ptr, client.shm_size);
            sys::close(client.shm_fd);
        }

        // Unmap back buffer and framebuffer.
        sys::mem_unmap(self.back_ptr, self.fb_size);
        sys::mem_unmap(self.fb_ptr, self.fb_size);
        io::close(self.fb_fd);
    }
}

// ── Entry point ──────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn main(_args: &[&str]) -> i32 {
    let mut comp = match Compositor::init() {
        Some(c) => c,
        None => return 1,
    };

    // Set stdin to raw mode.
    let mut orig_termios = Termios {
        iflag: 0,
        oflag: 0,
        cflag: 0,
        lflag: 0,
        cc: [0; 32],
    };
    io::ioctl(
        0,
        TCGETS as usize,
        &mut orig_termios as *mut Termios as usize,
    );
    let mut raw = orig_termios;
    raw.lflag &= !(ICANON | ECHO);
    io::ioctl(0, TCSETS as usize, &raw as *const Termios as usize);

    // Spawn initial clients.
    // Sysmon: top-left corner, 400x300.
    let sysmon_w = comp.fb_width.min(400);
    let sysmon_h = comp.fb_height.min(300);
    comp.spawn_client("/bin/sysmon", 20, 40, sysmon_w, sysmon_h);

    // Terminal: offset from sysmon, 640x400.
    let term_w = comp.fb_width.min(640);
    let term_h = comp.fb_height.min(400);
    comp.spawn_client("/bin/terminal", 440, 40, term_w, term_h);

    // Main loop.
    loop {
        comp.poll_keyboard();
        comp.poll_mouse();
        comp.poll_clients();
        comp.composite();
        sys::sleep_ms(FRAME_MS);
    }

    // Note: in practice, the compositor runs until the system shuts down.
    // If we ever add a quit mechanism:
    // io::ioctl(0, TCSETS as usize, &orig_termios as *const Termios as usize);
    // comp.shutdown();
    // 0
}
