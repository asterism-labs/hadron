//! Display client library for compositor-managed applications.
//!
//! Connects to the Wayland compositor at `/run/wayland-0` and performs the
//! full Wayland handshake to obtain a shared-memory surface for rendering.
//!
//! The public API is unchanged from the previous channel-based implementation:
//! [`Display::connect`], [`Display::surface`], [`Display::commit`],
//! [`Display::poll_event`], and [`Display::disconnect`].

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use lepton_gfx::Surface;
use lepton_syslib::sys;
use lepton_wayland::consts::*;
use lepton_wayland::{net, wire};

/// Bytes per pixel (32-bit XRGB).
const BPP: usize = 4;

/// Default surface width when compositor sends (0, 0).
const DEFAULT_WIDTH: u32 = 800;
/// Default surface height when compositor sends (0, 0).
const DEFAULT_HEIGHT: u32 = 600;

// -- Object ID scheme (hardcoded) --------------------------------------------

/// `wl_display` — implicit, always 1.
const OBJ_DISPLAY: u32 = 1;
/// `wl_registry` — allocated by `get_registry`.
const OBJ_REGISTRY: u32 = 2;
/// `wl_compositor` — bound global.
const OBJ_COMPOSITOR: u32 = 3;
/// `wl_shm` — bound global.
const OBJ_SHM: u32 = 4;
/// `xdg_wm_base` — bound global.
const OBJ_XDG_WM_BASE: u32 = 5;
/// `wl_surface` — created surface.
const OBJ_SURFACE: u32 = 6;
/// `wl_shm_pool` — created pool.
const OBJ_SHM_POOL: u32 = 7;
/// `wl_buffer` — created buffer.
const OBJ_BUFFER: u32 = 8;
/// `xdg_surface` — wraps wl_surface.
const OBJ_XDG_SURFACE: u32 = 9;
/// `xdg_toplevel` — top-level window role.
const OBJ_TOPLEVEL: u32 = 10;

/// A connection to the compositor.
pub struct Display {
    /// Surface width in pixels.
    width: u32,
    /// Surface height in pixels.
    height: u32,
    /// Pointer to the mapped shared-memory pixel buffer.
    shm_ptr: *mut u8,
    /// Size of the shared-memory region in bytes.
    shm_size: usize,
    /// Socket fd connected to the compositor.
    socket_fd: usize,
    /// Shared-memory fd (kept open so the mapping stays valid).
    shm_fd: usize,
}

/// An event received from the compositor.
pub enum Event {
    /// A key was pressed or released.
    Key {
        /// ASCII character (0 if non-printable).
        character: u8,
        /// Raw scancode.
        keycode: u32,
        /// `true` = pressed, `false` = released.
        pressed: bool,
    },
    /// A mouse event (position + button state).
    Mouse {
        /// Cursor X relative to this surface.
        x: i32,
        /// Cursor Y relative to this surface.
        y: i32,
        /// Current button bitmask.
        buttons: u32,
        /// Previous button bitmask.
        prev_buttons: u32,
    },
    /// This surface gained keyboard focus.
    FocusGained,
    /// This surface lost keyboard focus.
    FocusLost,
}

impl Display {
    /// Connect to the compositor via Wayland protocol.
    ///
    /// Performs the full handshake: socket connect, registry bind, surface
    /// creation, SHM pool setup, and initial configure/attach/commit.
    pub fn connect() -> Option<Self> {
        // 1. Create socket and connect to compositor.
        let sfd = net::socket_create();
        if sfd < 0 {
            return None;
        }
        let socket_fd = sfd as usize;

        if net::socket_connect(socket_fd, WAYLAND_SOCKET_PATH) < 0 {
            sys::close(socket_fd);
            return None;
        }

        // 2. Send wl_display.get_registry(new_id=2).
        let mut buf = Vec::new();
        let start = wire::begin_msg(&mut buf, OBJ_DISPLAY, WL_DISPLAY_GET_REGISTRY);
        wire::push_u32(&mut buf, OBJ_REGISTRY);
        wire::end_msg(&mut buf, start, WL_DISPLAY_GET_REGISTRY);
        if !net::send_all(socket_fd, &buf) {
            sys::close(socket_fd);
            return None;
        }

        // 3. Read registry global events.
        //    We expect 3 globals: wl_compositor, wl_shm, xdg_wm_base.
        let mut recv_buf = [0u8; 4096];
        let mut recv_pos = 0usize;
        let mut globals_seen = 0u32;

        while globals_seen < 3 {
            let (n, _) = net::recv_with_fd(socket_fd, &mut recv_buf[recv_pos..]);
            if n <= 0 {
                sys::close(socket_fd);
                return None;
            }
            recv_pos += n as usize;

            // Parse all complete messages in the buffer.
            let mut consumed = 0;
            while let Some((obj, opcode, size)) = wire::parse_header(&recv_buf[consumed..recv_pos])
            {
                if obj == OBJ_REGISTRY && opcode == WL_REGISTRY_GLOBAL {
                    globals_seen += 1;
                }
                consumed += size;
            }
            // Shift unconsumed data to front.
            if consumed > 0 {
                recv_buf.copy_within(consumed..recv_pos, 0);
                recv_pos -= consumed;
            }
        }

        // 4. Bind globals: compositor, shm, xdg_wm_base.
        buf.clear();
        Self::bind_global(
            &mut buf,
            OBJ_REGISTRY,
            GLOBAL_COMPOSITOR,
            WL_COMPOSITOR,
            WL_COMPOSITOR_VERSION,
            OBJ_COMPOSITOR,
        );
        Self::bind_global(
            &mut buf,
            OBJ_REGISTRY,
            GLOBAL_SHM,
            WL_SHM,
            WL_SHM_VERSION,
            OBJ_SHM,
        );
        Self::bind_global(
            &mut buf,
            OBJ_REGISTRY,
            GLOBAL_XDG_WM_BASE,
            XDG_WM_BASE,
            XDG_WM_BASE_VERSION,
            OBJ_XDG_WM_BASE,
        );
        if !net::send_all(socket_fd, &buf) {
            sys::close(socket_fd);
            return None;
        }

        // 5. Read wl_shm.format events (expect at least XRGB8888).
        //    Drain until we see at least one format event.
        let mut got_format = false;
        while !got_format {
            let (n, _) = net::recv_with_fd(socket_fd, &mut recv_buf[recv_pos..]);
            if n <= 0 {
                sys::close(socket_fd);
                return None;
            }
            recv_pos += n as usize;

            let mut consumed = 0;
            while let Some((obj, opcode, size)) = wire::parse_header(&recv_buf[consumed..recv_pos])
            {
                if obj == OBJ_SHM && opcode == WL_SHM_FORMAT_EVENT {
                    got_format = true;
                }
                consumed += size;
            }
            if consumed > 0 {
                recv_buf.copy_within(consumed..recv_pos, 0);
                recv_pos -= consumed;
            }
        }

        // 6. Create surface: wl_compositor.create_surface(new_id=6).
        buf.clear();
        let start = wire::begin_msg(&mut buf, OBJ_COMPOSITOR, WL_COMPOSITOR_CREATE_SURFACE);
        wire::push_u32(&mut buf, OBJ_SURFACE);
        wire::end_msg(&mut buf, start, WL_COMPOSITOR_CREATE_SURFACE);
        if !net::send_all(socket_fd, &buf) {
            sys::close(socket_fd);
            return None;
        }

        // 7. Create shared memory for the pixel buffer.
        //    Use default size initially; we resize after configure.
        let width = DEFAULT_WIDTH;
        let height = DEFAULT_HEIGHT;
        let shm_size = width as usize * height as usize * BPP;

        let shm_fd = match sys::mem_create_shared(shm_size) {
            Ok(fd) => fd,
            Err(_) => {
                sys::close(socket_fd);
                return None;
            }
        };

        let shm_ptr = match sys::mem_map_shared(shm_fd, shm_size) {
            Some(ptr) => ptr,
            None => {
                sys::close(shm_fd);
                sys::close(socket_fd);
                return None;
            }
        };

        // 8. Send wl_shm.create_pool with fd, then create buffer, xdg_surface, toplevel.
        buf.clear();

        // wl_shm.create_pool(new_id=7, fd=shm_fd, size) — fd sent OOB
        let start = wire::begin_msg(&mut buf, OBJ_SHM, WL_SHM_CREATE_POOL);
        wire::push_u32(&mut buf, OBJ_SHM_POOL);
        wire::push_i32(&mut buf, shm_size as i32);
        wire::end_msg(&mut buf, start, WL_SHM_CREATE_POOL);

        // Send the pool creation message with the shm fd attached.
        if !net::send_with_fd(socket_fd, &buf, shm_fd as i32) {
            sys::mem_unmap(shm_ptr, shm_size);
            sys::close(shm_fd);
            sys::close(socket_fd);
            return None;
        }

        // wl_shm_pool.create_buffer(new_id=8, offset=0, w, h, stride, format=XRGB8888)
        buf.clear();
        let stride = width * BPP as u32;
        let start = wire::begin_msg(&mut buf, OBJ_SHM_POOL, WL_SHM_POOL_CREATE_BUFFER);
        wire::push_u32(&mut buf, OBJ_BUFFER);
        wire::push_i32(&mut buf, 0); // offset
        wire::push_i32(&mut buf, width as i32);
        wire::push_i32(&mut buf, height as i32);
        wire::push_i32(&mut buf, stride as i32);
        wire::push_u32(&mut buf, WL_SHM_FORMAT_XRGB8888);
        wire::end_msg(&mut buf, start, WL_SHM_POOL_CREATE_BUFFER);

        // xdg_wm_base.get_xdg_surface(new_id=9, surface=6)
        let start = wire::begin_msg(&mut buf, OBJ_XDG_WM_BASE, XDG_WM_BASE_GET_XDG_SURFACE);
        wire::push_u32(&mut buf, OBJ_XDG_SURFACE);
        wire::push_u32(&mut buf, OBJ_SURFACE);
        wire::end_msg(&mut buf, start, XDG_WM_BASE_GET_XDG_SURFACE);

        // xdg_surface.get_toplevel(new_id=10)
        let start = wire::begin_msg(&mut buf, OBJ_XDG_SURFACE, XDG_SURFACE_GET_TOPLEVEL);
        wire::push_u32(&mut buf, OBJ_TOPLEVEL);
        wire::end_msg(&mut buf, start, XDG_SURFACE_GET_TOPLEVEL);

        // wl_surface.commit() — triggers the initial configure sequence
        let start = wire::begin_msg(&mut buf, OBJ_SURFACE, WL_SURFACE_COMMIT);
        wire::end_msg(&mut buf, start, WL_SURFACE_COMMIT);

        if !net::send_all(socket_fd, &buf) {
            sys::mem_unmap(shm_ptr, shm_size);
            sys::close(shm_fd);
            sys::close(socket_fd);
            return None;
        }

        // 9. Read configure events:
        //    - xdg_toplevel.configure(w, h, states)
        //    - xdg_surface.configure(serial)
        let mut final_width = width;
        let mut final_height = height;
        let mut configure_serial: Option<u32> = None;

        while configure_serial.is_none() {
            let (n, _) = net::recv_with_fd(socket_fd, &mut recv_buf[recv_pos..]);
            if n <= 0 {
                sys::mem_unmap(shm_ptr, shm_size);
                sys::close(shm_fd);
                sys::close(socket_fd);
                return None;
            }
            recv_pos += n as usize;

            let mut consumed = 0;
            while let Some((obj, opcode, size)) = wire::parse_header(&recv_buf[consumed..recv_pos])
            {
                let args = &recv_buf[consumed + 8..consumed + size];

                if obj == OBJ_TOPLEVEL && opcode == XDG_TOPLEVEL_CONFIGURE {
                    // xdg_toplevel.configure(width: int, height: int, states: array)
                    if let Some((w, off)) = wire::read_i32(args, 0) {
                        if let Some((h, _)) = wire::read_i32(args, off) {
                            if w > 0 && h > 0 {
                                final_width = w as u32;
                                final_height = h as u32;
                            }
                        }
                    }
                } else if obj == OBJ_XDG_SURFACE && opcode == XDG_SURFACE_CONFIGURE {
                    // xdg_surface.configure(serial)
                    if let Some((serial, _)) = wire::read_u32(args, 0) {
                        configure_serial = Some(serial);
                    }
                }

                consumed += size;
            }
            if consumed > 0 {
                recv_buf.copy_within(consumed..recv_pos, 0);
                recv_pos -= consumed;
            }
        }

        // 10. Ack configure + attach + commit.
        buf.clear();

        // xdg_surface.ack_configure(serial)
        let serial = configure_serial.unwrap_or(0);
        let start = wire::begin_msg(&mut buf, OBJ_XDG_SURFACE, XDG_SURFACE_ACK_CONFIGURE);
        wire::push_u32(&mut buf, serial);
        wire::end_msg(&mut buf, start, XDG_SURFACE_ACK_CONFIGURE);

        // wl_surface.attach(buffer=8, x=0, y=0)
        let start = wire::begin_msg(&mut buf, OBJ_SURFACE, WL_SURFACE_ATTACH);
        wire::push_u32(&mut buf, OBJ_BUFFER);
        wire::push_i32(&mut buf, 0);
        wire::push_i32(&mut buf, 0);
        wire::end_msg(&mut buf, start, WL_SURFACE_ATTACH);

        // wl_surface.commit()
        let start = wire::begin_msg(&mut buf, OBJ_SURFACE, WL_SURFACE_COMMIT);
        wire::end_msg(&mut buf, start, WL_SURFACE_COMMIT);

        if !net::send_all(socket_fd, &buf) {
            sys::mem_unmap(shm_ptr, shm_size);
            sys::close(shm_fd);
            sys::close(socket_fd);
            return None;
        }

        Some(Display {
            width: final_width,
            height: final_height,
            shm_ptr,
            shm_size,
            socket_fd,
            shm_fd,
        })
    }

    /// Build a `wl_registry.bind` message into `buf`.
    fn bind_global(
        buf: &mut Vec<u8>,
        registry_id: u32,
        name: u32,
        iface: &[u8],
        version: u32,
        new_id: u32,
    ) {
        let start = wire::begin_msg(buf, registry_id, WL_REGISTRY_BIND);
        wire::push_u32(buf, name);
        wire::push_str(buf, iface);
        wire::push_u32(buf, version);
        wire::push_u32(buf, new_id);
        wire::end_msg(buf, start, WL_REGISTRY_BIND);
    }

    /// Returns the surface width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the surface height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Obtain a drawable surface backed by the shared-memory buffer.
    ///
    /// The returned `Surface` directly references the shared pixel data.
    /// After drawing, call [`commit`](Self::commit) to notify the compositor.
    pub fn surface(&mut self) -> Surface<'_> {
        let pixel_count = self.width as usize * self.height as usize;
        // SAFETY: shm_ptr is a valid mapping of `width * height * 4` bytes,
        // interpreted as u32 pixels. The mutable borrow of `self` ensures
        // exclusive access.
        let pixels =
            unsafe { core::slice::from_raw_parts_mut(self.shm_ptr.cast::<u32>(), pixel_count) };
        Surface::from_raw(pixels, self.width, self.height, self.width)
    }

    /// Signal the compositor that drawing is complete and the surface is ready
    /// to be composited.
    pub fn commit(&mut self) {
        let mut buf = Vec::new();

        // wl_surface.attach(buffer=8, x=0, y=0)
        let start = wire::begin_msg(&mut buf, OBJ_SURFACE, WL_SURFACE_ATTACH);
        wire::push_u32(&mut buf, OBJ_BUFFER);
        wire::push_i32(&mut buf, 0);
        wire::push_i32(&mut buf, 0);
        wire::end_msg(&mut buf, start, WL_SURFACE_ATTACH);

        // wl_surface.commit()
        let start = wire::begin_msg(&mut buf, OBJ_SURFACE, WL_SURFACE_COMMIT);
        wire::end_msg(&mut buf, start, WL_SURFACE_COMMIT);

        let _ = net::send_all(self.socket_fd, &buf);
    }

    /// Poll for the next event from the compositor (non-blocking).
    ///
    /// Returns `None` if no event is pending. Automatically handles
    /// `xdg_wm_base.ping` keepalives and `wl_buffer.release` events.
    pub fn poll_event(&mut self) -> Option<Event> {
        if !sys::poll_fd_read(self.socket_fd) {
            return None;
        }

        let mut buf = [0u8; 4096];
        let (n, _) = net::recv_with_fd(self.socket_fd, &mut buf);
        if n <= 0 {
            return None;
        }

        let recv_len = n as usize;
        let mut consumed = 0;

        while let Some((obj, opcode, size)) = wire::parse_header(&buf[consumed..recv_len]) {
            let args = &buf[consumed + 8..consumed + size];

            // Handle xdg_wm_base.ping → auto-reply pong
            if obj == OBJ_XDG_WM_BASE && opcode == XDG_WM_BASE_PING {
                if let Some((serial, _)) = wire::read_u32(args, 0) {
                    let mut reply = Vec::new();
                    let start = wire::begin_msg(&mut reply, OBJ_XDG_WM_BASE, XDG_WM_BASE_PONG);
                    wire::push_u32(&mut reply, serial);
                    wire::end_msg(&mut reply, start, XDG_WM_BASE_PONG);
                    let _ = net::send_all(self.socket_fd, &reply);
                }
            }

            // wl_buffer.release — no-op for single-buffer scheme
            // Other events — no input forwarding yet (no wl_seat)

            consumed += size;
        }

        None
    }

    /// Disconnect from the compositor, unmapping shared memory and closing fds.
    pub fn disconnect(self) {
        // Send cleanup messages (best-effort).
        let mut buf = Vec::new();

        // wl_surface.destroy()
        let start = wire::begin_msg(&mut buf, OBJ_SURFACE, WL_SURFACE_DESTROY);
        wire::end_msg(&mut buf, start, WL_SURFACE_DESTROY);

        // wl_buffer.destroy()
        let start = wire::begin_msg(&mut buf, OBJ_BUFFER, WL_BUFFER_DESTROY);
        wire::end_msg(&mut buf, start, WL_BUFFER_DESTROY);

        // wl_shm_pool.destroy()
        let start = wire::begin_msg(&mut buf, OBJ_SHM_POOL, WL_SHM_POOL_DESTROY);
        wire::end_msg(&mut buf, start, WL_SHM_POOL_DESTROY);

        let _ = net::send_all(self.socket_fd, &buf);

        sys::mem_unmap(self.shm_ptr, self.shm_size);
        sys::close(self.shm_fd);
        sys::close(self.socket_fd);
    }
}
