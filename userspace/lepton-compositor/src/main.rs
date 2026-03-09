//! Lepton compositor — Wayland WSI backend for Mesa lavapipe.
//!
//! Listens on `/run/wayland-0` (UNIX stream socket) and implements the minimal
//! Wayland protocol subset needed for Mesa lavapipe (software Vulkan):
//!
//! - `wl_display` (implicit, id=1)
//! - `wl_registry` (client-created)
//! - `wl_compositor` v4  (global name 1)
//! - `wl_shm` v1          (global name 2)
//! - `xdg_wm_base` v2     (global name 3)
//! - `wl_surface`, `wl_shm_pool`, `wl_buffer`, `wl_callback`
//! - `xdg_surface`, `xdg_toplevel`
//!
//! Surfaces are blitted from wl_shm buffers to `/dev/fb0` at ~60 fps.
//! Three display targets are supported via the single `/dev/fb0` interface:
//! VBox VGA, Bochs BGA, and QEMU VirtIO-GPU (all expose the same fb ioctl API).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use lepton_syslib::hadron_syscall::{
    CLOCK_MONOTONIC, FBIOBLANK, FBIODIRTY, FBIOGET_INFO, FbDirtyRect, FbInfo, POLLIN, PollFd,
    Timespec,
};
use lepton_syslib::{io, println, sys};

use lepton_wayland::net;
use lepton_wayland::wire as proto;

// ── Constants ────────────────────────────────────────────────────────────────

/// Socket path for the Wayland compositor.
const WAYLAND_SOCKET_PATH: &[u8] = b"/run/wayland-0";

/// Target frame time in milliseconds (~60 fps).
const FRAME_MS: u64 = 16;

/// wl_shm pixel format: 32-bit ARGB little-endian.
const WL_SHM_FORMAT_ARGB8888: u32 = 0;
/// wl_shm pixel format: 32-bit XRGB (opaque) little-endian.
const WL_SHM_FORMAT_XRGB8888: u32 = 1;

/// xdg_toplevel state: window has keyboard focus.
const XDG_TOPLEVEL_STATE_ACTIVATED: u32 = 4;

/// Background fill colour for uncovered framebuffer regions.
const BG_COLOR: u32 = 0x0020_2020;

// ── Pixel buffer ─────────────────────────────────────────────────────────────

/// Thin safe wrapper around an mmap'd pixel buffer.
struct PixelBuffer {
    ptr: *mut u8,
    size: usize,
}

impl PixelBuffer {
    /// # Safety
    /// `ptr` must be a valid, writable mapping of at least `size` bytes that
    /// remains valid for the lifetime of this `PixelBuffer`.
    unsafe fn new(ptr: *mut u8, size: usize) -> Self {
        Self { ptr, size }
    }

    fn pixels_mut(&self, pixel_count: usize) -> &mut [u32] {
        assert!(pixel_count * 4 <= self.size, "pixel buffer overflow");
        // SAFETY: see constructor invariant; only one mutable slice at a time.
        unsafe { core::slice::from_raw_parts_mut(self.ptr as *mut u32, pixel_count) }
    }

    fn as_ptr(&self) -> *mut u8 {
        self.ptr
    }
}

// ── Wayland object table ─────────────────────────────────────────────────────

/// Discriminant for entries in a client's object table.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ObjKind {
    Display,
    Registry,
    Compositor,
    Surface,
    Shm,
    ShmPool,
    Buffer,
    Callback,
    Region,
    XdgWmBase,
    XdgSurface,
    XdgToplevel,
    Deleted,
}

/// Per-surface compositor state.
struct SurfaceData {
    /// wl_buffer object ID currently attached but not yet committed.
    pending_buffer: Option<u32>,
    /// wl_buffer object ID from the last commit (what we're currently displaying).
    current_buffer: Option<u32>,
    /// wl_callback IDs waiting for the next frame.
    frame_callbacks: Vec<u32>,
    /// Whether we have already sent the initial xdg configure.
    xdg_configured: bool,
    /// Whether the client has ack'd the configure.
    xdg_acked: bool,
    /// x/y position on screen (compositor-assigned).
    x: i32,
    y: i32,
}

impl SurfaceData {
    fn new() -> Self {
        SurfaceData {
            pending_buffer: None,
            current_buffer: None,
            frame_callbacks: Vec::new(),
            xdg_configured: false,
            xdg_acked: false,
            x: 0,
            y: 0,
        }
    }
}

/// Per-shm-pool compositor state.
struct ShmPoolData {
    /// Mapped pointer (from sys_mem_map_shared).
    ptr: *mut u8,
    /// Mapped size in bytes.
    size: usize,
    /// Original fd (kept open so the mapping stays valid).
    fd: i32,
}

/// Per-buffer compositor state.
#[derive(Clone)]
struct BufferData {
    /// Which ShmPool object ID this buffer is in.
    pool_id: u32,
    /// Byte offset into the pool.
    offset: usize,
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
    /// Bytes per row.
    stride: u32,
    /// Pixel format (WL_SHM_FORMAT_*).
    format: u32,
}

/// One entry in a client's object table.
struct ObjEntry {
    id: u32,
    kind: ObjKind,
    // Inline per-kind payloads:
    surface: Option<SurfaceData>,
    shm_pool: Option<ShmPoolData>,
    buffer: Option<BufferData>,
    /// For XdgSurface: which wl_surface object ID it wraps.
    xdg_surface_surface_id: u32,
    /// For XdgToplevel: which xdg_surface object ID it wraps.
    xdg_toplevel_xdg_surface_id: u32,
}

impl ObjEntry {
    fn new(id: u32, kind: ObjKind) -> Self {
        ObjEntry {
            id,
            kind,
            surface: None,
            shm_pool: None,
            buffer: None,
            xdg_surface_surface_id: 0,
            xdg_toplevel_xdg_surface_id: 0,
        }
    }
}

// ── Client state ─────────────────────────────────────────────────────────────

/// Maximum Wayland read-buffer size (4 KiB covers most bursts).
const READ_BUF_CAP: usize = 4096;

/// Per-connected-client state.
struct Client {
    /// Connected socket file descriptor.
    fd: usize,
    /// Partially-received Wayland messages.
    read_buf: Vec<u8>,
    /// Outgoing event bytes not yet sent.
    write_buf: Vec<u8>,
    /// Flat object table (linear-searched; typical size < 20 entries).
    objects: Vec<ObjEntry>,
    /// Serial counter for configure events.
    next_serial: u32,
    /// Fd received in a previous recv that no handler consumed yet.
    /// Carried across dispatch cycles so it reaches the right message.
    deferred_fd: Option<usize>,
}

impl Client {
    fn new(fd: usize) -> Self {
        let mut c = Client {
            fd,
            read_buf: Vec::new(),
            write_buf: Vec::new(),
            objects: Vec::new(),
            next_serial: 1,
            deferred_fd: None,
        };
        // wl_display is always object 1.
        c.objects.push(ObjEntry::new(1, ObjKind::Display));
        c
    }

    /// Look up the kind for an object ID. Returns Deleted if not found.
    fn obj_kind(&self, id: u32) -> ObjKind {
        for e in &self.objects {
            if e.id == id {
                return e.kind;
            }
        }
        ObjKind::Deleted
    }

    /// Register a new object with the given ID and kind.
    fn register(&mut self, id: u32, kind: ObjKind) -> &mut ObjEntry {
        // Index-based search avoids borrow-checker conflict between the loop
        // borrow and the subsequent push/index-mutate.
        let mut found: Option<usize> = None;
        for i in 0..self.objects.len() {
            if self.objects[i].id == id && self.objects[i].kind == ObjKind::Deleted {
                found = Some(i);
                break;
            }
        }
        if let Some(idx) = found {
            self.objects[idx] = ObjEntry::new(id, kind);
            return &mut self.objects[idx];
        }
        self.objects.push(ObjEntry::new(id, kind));
        self.objects.last_mut().unwrap()
    }

    fn destroy_obj(&mut self, id: u32) {
        for e in &mut self.objects {
            if e.id == id {
                // Clean up ShmPool mapping.
                if let Some(p) = e.shm_pool.take() {
                    sys::mem_unmap(p.ptr, p.size);
                    sys::close(p.fd as usize);
                }
                e.kind = ObjKind::Deleted;
                e.surface = None;
                e.buffer = None;
                return;
            }
        }
    }

    /// Allocate and return the next configure serial.
    fn next_serial(&mut self) -> u32 {
        let s = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1);
        s
    }

    // ── Event helpers ────────────────────────────────────────────────────────

    /// Queue `wl_registry.global(name, interface, version)` on `registry_id`.
    fn send_registry_global(&mut self, reg_id: u32, name: u32, iface: &[u8], version: u32) {
        let start = proto::begin_msg(&mut self.write_buf, reg_id, 0 /* global */);
        proto::push_u32(&mut self.write_buf, name);
        proto::push_str(&mut self.write_buf, iface);
        proto::push_u32(&mut self.write_buf, version);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Queue `wl_shm.format(fmt)` on `shm_id`.
    fn send_shm_format(&mut self, shm_id: u32, fmt: u32) {
        let start = proto::begin_msg(&mut self.write_buf, shm_id, 0 /* format */);
        proto::push_u32(&mut self.write_buf, fmt);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Queue `xdg_toplevel.configure(width, height, states[])` on `toplevel_id`.
    fn send_xdg_toplevel_configure(
        &mut self,
        toplevel_id: u32,
        width: i32,
        height: i32,
        states: &[u32],
    ) {
        let start = proto::begin_msg(&mut self.write_buf, toplevel_id, 0 /* configure */);
        proto::push_i32(&mut self.write_buf, width);
        proto::push_i32(&mut self.write_buf, height);
        // states as wl_array of u32 values
        let state_bytes: &[u8] =
            unsafe { core::slice::from_raw_parts(states.as_ptr() as *const u8, states.len() * 4) };
        proto::push_array(&mut self.write_buf, state_bytes);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Queue `xdg_surface.configure(serial)` on `xdg_surface_id`.
    fn send_xdg_surface_configure(&mut self, xdg_surface_id: u32, serial: u32) {
        let start = proto::begin_msg(&mut self.write_buf, xdg_surface_id, 0 /* configure */);
        proto::push_u32(&mut self.write_buf, serial);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Queue `xdg_wm_base.ping(serial)` on `wm_base_id`.
    fn send_xdg_wm_base_ping(&mut self, wm_base_id: u32, serial: u32) {
        let start = proto::begin_msg(&mut self.write_buf, wm_base_id, 0 /* ping */);
        proto::push_u32(&mut self.write_buf, serial);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Queue `wl_callback.done(time_ms)` on `callback_id`.
    fn send_callback_done(&mut self, cb_id: u32, time_ms: u32) {
        let start = proto::begin_msg(&mut self.write_buf, cb_id, 0 /* done */);
        proto::push_u32(&mut self.write_buf, time_ms);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Queue `wl_display.delete_id(id)` on the display (object 1).
    fn send_delete_id(&mut self, del_id: u32) {
        let start = proto::begin_msg(&mut self.write_buf, 1, 1 /* delete_id */);
        proto::push_u32(&mut self.write_buf, del_id);
        proto::end_msg(&mut self.write_buf, start, 1);
    }

    /// Queue `wl_buffer.release()` on `buf_id`.
    fn send_buffer_release(&mut self, buf_id: u32) {
        let start = proto::begin_msg(&mut self.write_buf, buf_id, 0 /* release */);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Queue a `wl_display.error(obj, code, msg)` and mark client for removal.
    fn send_error(&mut self, obj_id: u32, code: u32, msg: &[u8]) {
        let start = proto::begin_msg(&mut self.write_buf, 1, 0 /* error */);
        proto::push_u32(&mut self.write_buf, obj_id);
        proto::push_u32(&mut self.write_buf, code);
        proto::push_str(&mut self.write_buf, msg);
        proto::end_msg(&mut self.write_buf, start, 0);
    }

    /// Flush `write_buf` to the socket. Returns false if the peer disconnected.
    fn flush(&mut self) -> bool {
        if self.write_buf.is_empty() {
            return true;
        }
        let ok = net::send_all(self.fd, &self.write_buf);
        self.write_buf.clear();
        ok
    }
}

// ── Compositor ───────────────────────────────────────────────────────────────

/// Root compositor state.
struct Compositor {
    /// `/dev/fb0` file descriptor.
    fb_fd: usize,
    fb_width: u32,
    fb_height: u32,
    /// Bytes per row.
    fb_pitch: u32,
    /// Memory-mapped framebuffer.
    fb_buf: PixelBuffer,
    /// Back buffer for compositing (anonymous mmap, same size as fb).
    back_buf: PixelBuffer,
    /// Listening Unix socket.
    server_fd: usize,
    /// Connected Wayland clients.
    clients: Vec<Client>,
}

impl Compositor {
    /// Open `/dev/fb0`, mmap it, create the server socket.
    fn init() -> Option<Self> {
        // Open framebuffer.
        let fb_fd_r = io::open("/dev/fb0", 0);
        if fb_fd_r < 0 {
            println!("[compositor] failed to open /dev/fb0: {}", fb_fd_r);
            return None;
        }
        let fb_fd = fb_fd_r as usize;

        // Query framebuffer geometry.
        let mut info = core::mem::MaybeUninit::<FbInfo>::uninit();
        if io::ioctl(fb_fd, FBIOGET_INFO as usize, info.as_mut_ptr() as usize) < 0 {
            println!("[compositor] FBIOGET_INFO failed");
            sys::close(fb_fd);
            return None;
        }
        // SAFETY: kernel wrote a valid FbInfo on success.
        let info = unsafe { info.assume_init() };
        let fb_width = info.width;
        let fb_height = info.height;
        let fb_pitch = info.pitch;
        let fb_size = (fb_pitch * fb_height) as usize;
        println!(
            "[compositor] fb0: {}x{} pitch={}",
            fb_width, fb_height, fb_pitch
        );

        // Disable kernel fbcon so we own the screen.
        io::ioctl(fb_fd, FBIOBLANK as usize, 1);

        // Map framebuffer MMIO.
        let fb_ptr = sys::mem_map_device(fb_fd, fb_size)?;
        // SAFETY: valid mapping of fb_size bytes.
        let fb_buf = unsafe { PixelBuffer::new(fb_ptr, fb_size) };

        // Back buffer (anonymous writable memory, same size).
        let back_ptr = sys::mem_map(fb_size)?;
        // SAFETY: valid anonymous mapping.
        let back_buf = unsafe { PixelBuffer::new(back_ptr, fb_size) };

        // Create Wayland server socket.
        let sfd = net::socket_create();
        if sfd < 0 {
            println!("[compositor] socket() failed: {}", sfd);
            return None;
        }
        let server_fd = sfd as usize;

        // Unlink any stale socket file.
        lepton_syslib::hadron_syscall::wrappers::sys_vnode_unlink(
            WAYLAND_SOCKET_PATH.as_ptr() as usize,
            WAYLAND_SOCKET_PATH.len(),
        );

        let r = net::socket_bind(server_fd, WAYLAND_SOCKET_PATH);
        if r < 0 {
            println!("[compositor] bind() failed: {}", r);
            return None;
        }
        let r = net::socket_listen(server_fd, 16);
        if r < 0 {
            println!("[compositor] listen() failed: {}", r);
            return None;
        }

        println!("[compositor] listening on /run/wayland-0");
        Some(Compositor {
            fb_fd,
            fb_width,
            fb_height,
            fb_pitch,
            fb_buf,
            back_buf,
            server_fd,
            clients: Vec::new(),
        })
    }

    /// Fill the back-buffer and blit all committed client surfaces, then flip
    /// to the framebuffer. Also fires queued frame callbacks.
    fn composite(&mut self, time_ms: u32) {
        let pixels = (self.fb_pitch / 4) as usize * self.fb_height as usize;
        let back = self.back_buf.pixels_mut(pixels);

        // Clear background.
        back.fill(BG_COLOR);

        // Blit each client's committed surface.
        for client in &self.clients {
            for entry in &client.objects {
                if entry.kind != ObjKind::Surface {
                    continue;
                }
                let sd = match &entry.surface {
                    Some(s) if s.current_buffer.is_some() => s,
                    _ => continue,
                };
                let buf_id = sd.current_buffer.unwrap();

                // Find the buffer entry.
                let buf_data = client
                    .objects
                    .iter()
                    .find(|e| e.id == buf_id && e.kind == ObjKind::Buffer)
                    .and_then(|e| e.buffer.as_ref());
                let buf = match buf_data {
                    Some(b) => b,
                    None => continue,
                };

                // Find the shm pool.
                let pool = client
                    .objects
                    .iter()
                    .find(|e| e.id == buf.pool_id && e.kind == ObjKind::ShmPool)
                    .and_then(|e| e.shm_pool.as_ref());
                let pool = match pool {
                    Some(p) => p,
                    None => continue,
                };

                // Blit the surface to the back-buffer.
                blit(
                    back,
                    self.fb_pitch,
                    self.fb_height,
                    pool.ptr,
                    pool.size,
                    buf.offset,
                    buf.width,
                    buf.height,
                    buf.stride,
                    sd.x,
                    sd.y,
                );
            }
        }

        // Copy back-buffer → framebuffer MMIO.
        let byte_size = (self.fb_pitch * self.fb_height) as usize;
        unsafe {
            core::ptr::copy_nonoverlapping(self.back_buf.as_ptr(), self.fb_buf.as_ptr(), byte_size);
        }

        // Notify VirtIO-GPU driver that the full screen is dirty.
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

        // Fire frame callbacks on every surface that requested one.
        // Two-pass to avoid simultaneous mutable borrow of client.objects and
        // client.write_buf (send_callback_done / destroy_obj).
        for client in &mut self.clients {
            // Pass 1: drain callback IDs from all surfaces.
            let mut cbs_to_fire: Vec<u32> = Vec::new();
            for entry in &mut client.objects {
                if entry.kind == ObjKind::Surface {
                    if let Some(sd) = &mut entry.surface {
                        cbs_to_fire.extend(sd.frame_callbacks.drain(..));
                    }
                }
            }
            // Pass 2: send events and clean up objects.
            for cb_id in cbs_to_fire {
                client.send_callback_done(cb_id, time_ms);
                client.send_delete_id(cb_id);
                client.destroy_obj(cb_id);
            }
        }
    }

    /// Process one frame: accept connections, read client messages, composite,
    /// flush outputs.
    fn frame(&mut self) {
        // ── poll all fds ────────────────────────────────────────────────────
        let mut poll_fds: Vec<PollFd> = Vec::with_capacity(1 + self.clients.len());
        poll_fds.push(PollFd {
            fd: self.server_fd as u32,
            events: POLLIN as u16,
            revents: 0,
        });
        for c in &self.clients {
            poll_fds.push(PollFd {
                fd: c.fd as u32,
                events: POLLIN as u16,
                revents: 0,
            });
        }

        lepton_syslib::hadron_syscall::wrappers::sys_event_wait_many(
            poll_fds.as_mut_ptr() as usize,
            poll_fds.len(),
            FRAME_MS as usize,
        );

        // ── accept new connections ───────────────────────────────────────────
        if poll_fds[0].revents & POLLIN as u16 != 0 {
            let new_fd = net::socket_accept(self.server_fd);
            if new_fd >= 0 {
                println!("[compositor] client {} connected", new_fd);
                self.clients.push(Client::new(new_fd as usize));
            }
        }

        // ── read + dispatch client messages ─────────────────────────────────
        let mut disconnected: Vec<usize> = Vec::new();
        for (i, pfd) in poll_fds[1..].iter().enumerate() {
            if pfd.revents & POLLIN as u16 == 0 {
                continue;
            }
            let client = &mut self.clients[i];
            let mut tmp = [0u8; READ_BUF_CAP];
            let (n, recv_fd) = net::recv_with_fd(client.fd, &mut tmp);
            if n <= 0 {
                disconnected.push(i);
                continue;
            }
            client.read_buf.extend_from_slice(&tmp[..n as usize]);

            // Dispatch all complete messages in the read buffer.
            let recv_fd_opt = if recv_fd >= 0 {
                Some(recv_fd as usize)
            } else {
                None
            };
            // Merge any deferred fd from a previous recv cycle with the newly received fd.
            let mut pending_fd = recv_fd_opt.or(client.deferred_fd.take());
            let mut consumed = 0usize;
            let buf_snapshot: Vec<u8> = client.read_buf.clone();
            while let Some((obj_id, opcode, msg_size)) =
                proto::parse_header(&buf_snapshot[consumed..])
            {
                let args = &buf_snapshot[consumed + 8..consumed + msg_size];
                dispatch(client, obj_id, opcode, args, &mut pending_fd);
                consumed += msg_size;
            }
            // Store unconsumed fd for the next dispatch cycle instead of closing it.
            client.deferred_fd = pending_fd;
            // Discard the bytes we processed.
            {
                let remaining = client.read_buf.len().saturating_sub(consumed);
                let new_len = client.read_buf.len() - consumed;
                client.read_buf.copy_within(consumed.., 0);
                client.read_buf.truncate(new_len);
                let _ = remaining;
            }
        }

        // Remove disconnected clients (iterate in reverse to preserve indices).
        for i in disconnected.into_iter().rev() {
            let c = self.clients.remove(i);
            println!("[compositor] client {} disconnected", c.fd);
            if let Some(fd) = c.deferred_fd {
                sys::close(fd);
            }
            sys::close(c.fd);
        }

        // ── composite and flush ──────────────────────────────────────────────
        let now_ms = monotonic_ms();
        self.composite(now_ms as u32);

        for client in &mut self.clients {
            client.flush();
        }
    }
}

// ── Blit helper ──────────────────────────────────────────────────────────────

/// Blit pixels from a SHM pool slice onto the back-buffer at `(dst_x, dst_y)`.
///
/// Clips to the framebuffer dimensions. Supports ARGB8888 and XRGB8888.
fn blit(
    back: &mut [u32],
    fb_pitch: u32,
    fb_height: u32,
    pool_ptr: *mut u8,
    pool_size: usize,
    src_offset: usize,
    src_width: u32,
    src_height: u32,
    src_stride: u32,
    dst_x: i32,
    dst_y: i32,
) {
    let fb_stride = (fb_pitch / 4) as i32; // pixels per row
    let fb_h = fb_height as i32;
    for row in 0..src_height as i32 {
        let dy = dst_y + row;
        if dy < 0 || dy >= fb_h {
            continue;
        }
        let dy = dy as usize;
        for col in 0..src_width as i32 {
            let dx = dst_x + col;
            if dx < 0 || dx >= fb_stride {
                continue;
            }
            let dx = dx as usize;
            let src_byte = src_offset + (row as usize) * (src_stride as usize) + col as usize * 4;
            if src_byte + 4 > pool_size {
                continue;
            }
            // SAFETY: bounds checked above; pool_ptr is a valid SHM mapping.
            let px = unsafe {
                u32::from_le_bytes([
                    *pool_ptr.add(src_byte),
                    *pool_ptr.add(src_byte + 1),
                    *pool_ptr.add(src_byte + 2),
                    *pool_ptr.add(src_byte + 3),
                ])
            };
            back[dy * fb_stride as usize + dx] = px;
        }
    }
}

// ── Message dispatcher ───────────────────────────────────────────────────────

/// Dispatch one parsed Wayland request to the appropriate handler.
fn dispatch(client: &mut Client, obj_id: u32, opcode: u16, args: &[u8], recv_fd: &mut Option<usize>) {
    let kind = client.obj_kind(obj_id);
    match (kind, opcode) {
        // wl_display
        (ObjKind::Display, 0) => handle_display_sync(client, args),
        (ObjKind::Display, 1) => handle_display_get_registry(client, args),
        // wl_registry
        (ObjKind::Registry, 0) => handle_registry_bind(client, obj_id, args),
        // wl_compositor
        (ObjKind::Compositor, 0) => handle_compositor_create_surface(client, args),
        (ObjKind::Compositor, 1) => handle_compositor_create_region(client, args),
        // wl_surface
        (ObjKind::Surface, 0) => handle_surface_destroy(client, obj_id),
        (ObjKind::Surface, 1) => handle_surface_attach(client, obj_id, args),
        (ObjKind::Surface, 2) => { /* damage — no-op for software path */ }
        (ObjKind::Surface, 3) => handle_surface_frame(client, obj_id, args),
        (ObjKind::Surface, 4) | (ObjKind::Surface, 5) => { /* set_opaque/input_region — no-op */ }
        (ObjKind::Surface, 6) => handle_surface_commit(client, obj_id),
        (ObjKind::Surface, 9) => { /* damage_buffer — no-op */ }
        // wl_shm
        (ObjKind::Shm, 0) => handle_shm_create_pool(client, args, recv_fd),
        // wl_shm_pool
        (ObjKind::ShmPool, 0) => handle_shm_pool_create_buffer(client, obj_id, args),
        (ObjKind::ShmPool, 1) => handle_shm_pool_destroy(client, obj_id),
        (ObjKind::ShmPool, 2) => handle_shm_pool_resize(client, obj_id, args),
        // wl_buffer
        (ObjKind::Buffer, 0) => handle_buffer_destroy(client, obj_id),
        // wl_region — stub; accept and ignore all requests
        (ObjKind::Region, _) => {}
        // xdg_wm_base
        (ObjKind::XdgWmBase, 0) => { /* destroy */ }
        (ObjKind::XdgWmBase, 1) => { /* create_positioner — stub */ }
        (ObjKind::XdgWmBase, 2) => handle_xdg_wm_base_get_xdg_surface(client, args),
        (ObjKind::XdgWmBase, 3) => { /* pong — discard */ }
        // xdg_surface
        (ObjKind::XdgSurface, 0) => handle_xdg_surface_destroy(client, obj_id),
        (ObjKind::XdgSurface, 1) => handle_xdg_surface_get_toplevel(client, obj_id, args),
        (ObjKind::XdgSurface, 2) => { /* get_popup — stub */ }
        (ObjKind::XdgSurface, 3) => { /* set_window_geometry — no-op */ }
        (ObjKind::XdgSurface, 4) => handle_xdg_surface_ack_configure(client, obj_id, args),
        // xdg_toplevel
        (ObjKind::XdgToplevel, 0) => handle_xdg_toplevel_destroy(client, obj_id),
        (ObjKind::XdgToplevel, 1) => { /* set_parent — no-op */ }
        (ObjKind::XdgToplevel, 2) => { /* set_title — no-op */ }
        (ObjKind::XdgToplevel, 3) => { /* set_app_id — no-op */ }
        (ObjKind::XdgToplevel, 4..=12) => { /* misc hints — no-op */ }
        // Ignore unknown
        _ => {}
    }
}

// ── Handler implementations ──────────────────────────────────────────────────

/// `wl_display.sync(callback_id)` — immediately complete the callback.
fn handle_display_sync(client: &mut Client, args: &[u8]) {
    let (cb_id, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    client.register(cb_id, ObjKind::Callback);
    client.send_callback_done(cb_id, 0);
    client.send_delete_id(cb_id);
    client.destroy_obj(cb_id);
}

/// `wl_display.get_registry(registry_id)` — announce all globals.
fn handle_display_get_registry(client: &mut Client, args: &[u8]) {
    let (reg_id, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    client.register(reg_id, ObjKind::Registry);
    // Announce the three globals in order.
    client.send_registry_global(reg_id, 1, b"wl_compositor", 4);
    client.send_registry_global(reg_id, 2, b"wl_shm", 1);
    client.send_registry_global(reg_id, 3, b"xdg_wm_base", 2);
}

/// `wl_registry.bind(name, interface, version, new_id)`.
///
/// Wire format: uint32 name, string interface, uint32 version, uint32 new_id.
fn handle_registry_bind(client: &mut Client, _reg_id: u32, args: &[u8]) {
    let (name, off) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    let (_iface, off) = match proto::read_str(args, off) {
        Some(v) => v,
        None => return,
    };
    let (_version, off) = match proto::read_u32(args, off) {
        Some(v) => v,
        None => return,
    };
    let (new_id, _) = match proto::read_u32(args, off) {
        Some(v) => v,
        None => return,
    };

    match name {
        1 => {
            // wl_compositor
            client.register(new_id, ObjKind::Compositor);
        }
        2 => {
            // wl_shm — also send format announcements
            client.register(new_id, ObjKind::Shm);
            client.send_shm_format(new_id, WL_SHM_FORMAT_ARGB8888);
            client.send_shm_format(new_id, WL_SHM_FORMAT_XRGB8888);
        }
        3 => {
            // xdg_wm_base — send a ping immediately so client is alive
            client.register(new_id, ObjKind::XdgWmBase);
            let serial = client.next_serial();
            client.send_xdg_wm_base_ping(new_id, serial);
        }
        _ => {}
    }
}

/// `wl_compositor.create_surface(id)`.
fn handle_compositor_create_surface(client: &mut Client, args: &[u8]) {
    let (id, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    let e = client.register(id, ObjKind::Surface);
    e.surface = Some(SurfaceData::new());
}

/// `wl_compositor.create_region(id)` — stub: just register.
fn handle_compositor_create_region(client: &mut Client, args: &[u8]) {
    let (id, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    client.register(id, ObjKind::Region);
}

/// `wl_surface.destroy()`.
fn handle_surface_destroy(client: &mut Client, id: u32) {
    client.destroy_obj(id);
}

/// `wl_surface.attach(buffer_id, x, y)`.
fn handle_surface_attach(client: &mut Client, surf_id: u32, args: &[u8]) {
    let (buf_id, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    for e in &mut client.objects {
        if e.id == surf_id && e.kind == ObjKind::Surface {
            if let Some(sd) = &mut e.surface {
                sd.pending_buffer = if buf_id == 0 { None } else { Some(buf_id) };
            }
            break;
        }
    }
}

/// `wl_surface.frame(callback_id)`.
fn handle_surface_frame(client: &mut Client, surf_id: u32, args: &[u8]) {
    let (cb_id, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    client.register(cb_id, ObjKind::Callback);
    // Queue it on the surface's frame-callback list.
    for e in &mut client.objects {
        if e.id == surf_id && e.kind == ObjKind::Surface {
            if let Some(sd) = &mut e.surface {
                sd.frame_callbacks.push(cb_id);
            }
            break;
        }
    }
}

/// `wl_surface.commit()` — move pending state to current.
fn handle_surface_commit(client: &mut Client, surf_id: u32) {
    let mut prev_buf: Option<u32> = None;
    let mut new_buf: Option<u32> = None;

    for e in &mut client.objects {
        if e.id == surf_id && e.kind == ObjKind::Surface {
            if let Some(sd) = &mut e.surface {
                prev_buf = sd.current_buffer;
                new_buf = sd.pending_buffer;
                sd.current_buffer = sd.pending_buffer.take();
            }
            break;
        }
    }

    // Release the previous buffer if it changed.
    if let (Some(p), Some(n)) = (prev_buf, new_buf) {
        if p != n {
            client.send_buffer_release(p);
        }
    } else if let Some(p) = prev_buf {
        if new_buf.is_none() {
            client.send_buffer_release(p);
        }
    }
}

/// `wl_shm.create_pool(id, fd, size)`.
///
/// The fd arrives as SCM_RIGHTS in `recv_fd`. Wire args: [uint32 id][int32 size].
fn handle_shm_create_pool(client: &mut Client, args: &[u8], recv_fd: &mut Option<usize>) {
    let (id, off) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    let (size_i32, _) = match proto::read_i32(args, off) {
        Some(v) => v,
        None => return,
    };
    let size = size_i32.max(0) as usize;
    let fd = match recv_fd.take() {
        Some(f) => f,
        None => return, // fd is required
    };

    // Map the shared memory from the client.
    let ptr = match sys::mem_map_shared(fd, size) {
        Some(p) => p,
        None => {
            sys::close(fd);
            client.send_error(id, 0, b"shm_pool: failed to map shared memory");
            return;
        }
    };

    let e = client.register(id, ObjKind::ShmPool);
    e.shm_pool = Some(ShmPoolData {
        ptr,
        size,
        fd: fd as i32,
    });
}

/// `wl_shm_pool.create_buffer(id, offset, width, height, stride, format)`.
fn handle_shm_pool_create_buffer(client: &mut Client, pool_id: u32, args: &[u8]) {
    let (id, off) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    let (offset, off) = match proto::read_i32(args, off) {
        Some(v) => v,
        None => return,
    };
    let (width, off) = match proto::read_i32(args, off) {
        Some(v) => v,
        None => return,
    };
    let (height, off) = match proto::read_i32(args, off) {
        Some(v) => v,
        None => return,
    };
    let (stride, off) = match proto::read_i32(args, off) {
        Some(v) => v,
        None => return,
    };
    let (format, _) = match proto::read_u32(args, off) {
        Some(v) => v,
        None => return,
    };
    let e = client.register(id, ObjKind::Buffer);
    e.buffer = Some(BufferData {
        pool_id,
        offset: offset.max(0) as usize,
        width: width.max(0) as u32,
        height: height.max(0) as u32,
        stride: stride.max(0) as u32,
        format,
    });
}

/// `wl_shm_pool.destroy()`.
fn handle_shm_pool_destroy(client: &mut Client, id: u32) {
    client.destroy_obj(id);
}

/// `wl_shm_pool.resize(new_size)` — remap the pool.
fn handle_shm_pool_resize(client: &mut Client, id: u32, args: &[u8]) {
    let (new_size_i32, _) = match proto::read_i32(args, 0) {
        Some(v) => v,
        None => return,
    };
    let new_size = new_size_i32.max(0) as usize;
    let (old_ptr, old_size, fd) = {
        let e = client
            .objects
            .iter()
            .find(|e| e.id == id && e.kind == ObjKind::ShmPool);
        match e.and_then(|e| e.shm_pool.as_ref()) {
            Some(p) => (p.ptr, p.size, p.fd),
            None => return,
        }
    };
    sys::mem_unmap(old_ptr, old_size);
    let new_ptr = sys::mem_map_shared(fd as usize, new_size);
    for e in &mut client.objects {
        if e.id == id && e.kind == ObjKind::ShmPool {
            if let Some(p) = &mut e.shm_pool {
                match new_ptr {
                    Some(ptr) => {
                        p.ptr = ptr;
                        p.size = new_size;
                    }
                    None => {
                        // Mapping failed; mark pool as zero-size.
                        p.size = 0;
                    }
                }
            }
            break;
        }
    }
}

/// `wl_buffer.destroy()`.
fn handle_buffer_destroy(client: &mut Client, id: u32) {
    client.destroy_obj(id);
}

/// `xdg_wm_base.get_xdg_surface(id, surface_id)`.
fn handle_xdg_wm_base_get_xdg_surface(client: &mut Client, args: &[u8]) {
    let (id, off) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    let (surface_id, _) = match proto::read_u32(args, off) {
        Some(v) => v,
        None => return,
    };
    let e = client.register(id, ObjKind::XdgSurface);
    e.xdg_surface_surface_id = surface_id;
}

/// `xdg_surface.destroy()`.
fn handle_xdg_surface_destroy(client: &mut Client, id: u32) {
    client.destroy_obj(id);
}

/// `xdg_surface.get_toplevel(id)`.
///
/// Registers the toplevel and immediately sends the initial configure sequence:
/// `xdg_toplevel.configure(0,0,[ACTIVATED])` + `xdg_surface.configure(serial)`.
fn handle_xdg_surface_get_toplevel(client: &mut Client, xdg_surface_id: u32, args: &[u8]) {
    let (id, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    let e = client.register(id, ObjKind::XdgToplevel);
    e.xdg_toplevel_xdg_surface_id = xdg_surface_id;

    // Send initial configure: let client pick its own size (0x0).
    let states = [XDG_TOPLEVEL_STATE_ACTIVATED];
    client.send_xdg_toplevel_configure(id, 0, 0, &states);

    let serial = client.next_serial();
    client.send_xdg_surface_configure(xdg_surface_id, serial);

    // Mark surface as having received a configure.
    let surface_id = client
        .objects
        .iter()
        .find(|e| e.id == xdg_surface_id && e.kind == ObjKind::XdgSurface)
        .map(|e| e.xdg_surface_surface_id)
        .unwrap_or(0);
    if surface_id != 0 {
        for e in &mut client.objects {
            if e.id == surface_id && e.kind == ObjKind::Surface {
                if let Some(sd) = &mut e.surface {
                    sd.xdg_configured = true;
                }
                break;
            }
        }
    }
}

/// `xdg_surface.ack_configure(serial)`.
fn handle_xdg_surface_ack_configure(client: &mut Client, xdg_surface_id: u32, args: &[u8]) {
    let (_serial, _) = match proto::read_u32(args, 0) {
        Some(v) => v,
        None => return,
    };
    // Mark the underlying wl_surface as configured.
    let surface_id = client
        .objects
        .iter()
        .find(|e| e.id == xdg_surface_id && e.kind == ObjKind::XdgSurface)
        .map(|e| e.xdg_surface_surface_id)
        .unwrap_or(0);
    if surface_id != 0 {
        for e in &mut client.objects {
            if e.id == surface_id && e.kind == ObjKind::Surface {
                if let Some(sd) = &mut e.surface {
                    sd.xdg_acked = true;
                }
                break;
            }
        }
    }
}

/// `xdg_toplevel.destroy()`.
fn handle_xdg_toplevel_destroy(client: &mut Client, id: u32) {
    client.destroy_obj(id);
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Return the current monotonic clock time in milliseconds.
fn monotonic_ms() -> u64 {
    let mut ts = core::mem::MaybeUninit::<Timespec>::uninit();
    let ret = lepton_syslib::hadron_syscall::wrappers::sys_clock_gettime(
        CLOCK_MONOTONIC,
        ts.as_mut_ptr() as usize,
    );
    if ret < 0 {
        return 0;
    }
    // SAFETY: kernel wrote a valid Timespec on success.
    let ts = unsafe { ts.assume_init() };
    ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
extern "C" fn main(_argc: isize, _argv: *const *const u8) -> isize {
    println!("[compositor] Lepton Wayland compositor starting");

    let mut comp = match Compositor::init() {
        Some(c) => c,
        None => {
            println!("[compositor] init failed");
            return 1;
        }
    };

    // Launch a default terminal so the user has something to interact with.
    let ret = sys::spawn("/bin/terminal", &["/bin/terminal"]);
    if ret < 0 {
        println!("[compositor] failed to spawn terminal: errno {}", -ret);
    } else {
        println!("[compositor] spawned terminal (pid {})", ret);
    }

    println!("[compositor] running");
    loop {
        comp.frame();
    }
}
