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

use alloc::vec;
use alloc::vec::Vec;
use lepton_display_protocol::{self as proto, MESSAGE_SIZE, OP_COMMIT};
use lepton_gfx::Surface;
use lepton_syslib::hadron_syscall::{ECHO, FBIOGET_INFO, FbInfo, ICANON, TCGETS, TCSETS, Termios};
use lepton_syslib::{io, println, sys};

/// Bytes per pixel (32-bit color).
const BPP: usize = 4;

/// Frame interval in milliseconds (~60 fps).
const FRAME_MS: u64 = 16;

/// Background color for uncovered framebuffer regions.
const BG_COLOR: u32 = 0x0020_2020;

/// Border color for the focused surface.
const FOCUS_BORDER: u32 = 0x00FF_FFFF; // white

// ── Client state ─────────────────────────────────────────────────────

/// Per-client state tracked by the compositor.
struct ClientState {
    /// Child process PID.
    #[expect(dead_code, reason = "used for future waitpid / cleanup")]
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
}

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

        let fb_size = info.pitch as usize * info.height as usize;
        let fb_ptr = sys::mem_map_device(fb_fd, fb_size)?;

        // Allocate back buffer via anonymous mmap.
        let back_ptr = sys::mem_map(fb_size)?;

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

        // Blit each client surface (back-to-front).
        for (i, client) in self.clients.iter_mut().enumerate() {
            let src_pixel_count = client.width as usize * client.height as usize;
            // SAFETY: shm_ptr is a valid mapping of client's pixel buffer.
            let src_pixels = unsafe {
                core::slice::from_raw_parts_mut(client.shm_ptr as *mut u32, src_pixel_count)
            };
            let src = Surface::from_raw(src_pixels, client.width, client.height, client.width);
            back.blit(&src, client.x as i32, client.y as i32);

            // Draw 1px border on focused surface.
            if self.focused == Some(i) {
                let x = client.x;
                let y = client.y;
                let w = client.width;
                let h = client.height;
                // Top and bottom edges.
                if y > 0 {
                    back.hline(x.saturating_sub(1), y - 1, w + 2, FOCUS_BORDER);
                }
                back.hline(x.saturating_sub(1), y + h, w + 2, FOCUS_BORDER);
                // Left and right edges.
                if x > 0 {
                    back.vline(x - 1, y, h, FOCUS_BORDER);
                }
                back.vline(x + w, y, h, FOCUS_BORDER);
            }

            client.dirty = false;
        }

        // Flip: copy back buffer to framebuffer.
        // SAFETY: Both pointers are valid mappings of fb_size bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(self.back_ptr, self.fb_ptr, self.fb_size);
        }

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
    let client_w = comp.fb_width.min(400);
    let client_h = comp.fb_height.min(300);
    comp.spawn_client("/bin/sysmon", 20, 40, client_w, client_h);

    // Main loop.
    loop {
        comp.poll_keyboard();
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
