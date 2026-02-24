//! Display client library for compositor-managed applications.
//!
//! Supports two connection modes:
//! 1. **Legacy** (fd-passing at spawn): fd 3 = channel, fd 4 = shm.
//! 2. **Dynamic**: open `/dev/compositor`, send `CreateWindow`, receive
//!    `Configure` + shm fd via `channel_recv_fd`.
//!
//! [`Display::connect`] tries the legacy path first, then falls back to
//! the dynamic path.

#![no_std]

extern crate alloc;

use lepton_display_protocol::{
    self as proto, MESSAGE_SIZE, OP_CONFIGURE, OP_FOCUS_GAINED, OP_FOCUS_LOST, OP_KEYBOARD_INPUT,
    OP_MOUSE_INPUT,
};
use lepton_gfx::Surface;
use lepton_syslib::{io, sys};

/// Channel endpoint fd inherited from the compositor (legacy path).
const LEGACY_CHANNEL_FD: usize = 3;
/// Shared-memory fd inherited from the compositor (legacy path).
const LEGACY_SHM_FD: usize = 4;

/// Bytes per pixel (32-bit BGRA).
const BPP: usize = 4;

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
    /// Channel fd to the compositor.
    channel_fd: usize,
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
    /// Connect to the compositor.
    ///
    /// Tries the legacy path (fd 3/4 from spawn) first, then falls back to
    /// the dynamic path (opening `/dev/compositor`).
    pub fn connect() -> Option<Self> {
        // Try legacy: check if fd 3 has a pending Configure message.
        if sys::poll_fd_read(LEGACY_CHANNEL_FD) {
            if let Some(d) = Self::connect_legacy() {
                return Some(d);
            }
        }
        // Dynamic: open /dev/compositor.
        Self::connect_dynamic()
    }

    /// Legacy connection: read Configure from fd 3, map shm from fd 4.
    fn connect_legacy() -> Option<Self> {
        let mut buf = [0u8; MESSAGE_SIZE];
        let n = sys::channel_recv(LEGACY_CHANNEL_FD, &mut buf).ok()?;
        if n < MESSAGE_SIZE {
            return None;
        }

        if proto::peek_opcode(&buf) != OP_CONFIGURE {
            return None;
        }

        // SAFETY: We verified the opcode matches Configure.
        let cfg = unsafe { proto::cast_msg::<proto::Configure>(&buf) };
        let width = cfg.width;
        let height = cfg.height;

        let shm_size = width as usize * height as usize * BPP;
        let shm_ptr = sys::mem_map_shared(LEGACY_SHM_FD, shm_size)?;

        Some(Display {
            width,
            height,
            shm_ptr,
            shm_size,
            channel_fd: LEGACY_CHANNEL_FD,
            shm_fd: LEGACY_SHM_FD,
        })
    }

    /// Dynamic connection: open `/dev/compositor`, send `CreateWindow`,
    /// receive `Configure` + shm fd.
    fn connect_dynamic() -> Option<Self> {
        let fd = io::open("/dev/compositor", 3); // READ | WRITE
        if fd < 0 {
            return None;
        }
        let channel_fd = fd as usize;

        // Send CreateWindow request (0 = compositor chooses size).
        let create = proto::create_window(0, 0);
        let mut buf = [0u8; MESSAGE_SIZE];
        // SAFETY: CreateWindow is a 64-byte repr(C) message.
        unsafe { proto::encode_msg(&create, &mut buf) };
        if sys::channel_send(channel_fd, &buf).is_err() {
            sys::close(channel_fd);
            return None;
        }

        // Receive Configure + shm fd.
        let mut recv_buf = [0u8; MESSAGE_SIZE];
        let (n, shm_fd_opt) = sys::channel_recv_fd(channel_fd, &mut recv_buf).ok()?;
        if n < MESSAGE_SIZE || proto::peek_opcode(&recv_buf) != OP_CONFIGURE {
            sys::close(channel_fd);
            return None;
        }

        let shm_fd = shm_fd_opt?;

        // SAFETY: We verified the opcode matches Configure.
        let cfg = unsafe { proto::cast_msg::<proto::Configure>(&recv_buf) };
        let width = cfg.width;
        let height = cfg.height;

        let shm_size = width as usize * height as usize * BPP;
        let shm_ptr = sys::mem_map_shared(shm_fd, shm_size)?;

        Some(Display {
            width,
            height,
            shm_ptr,
            shm_size,
            channel_fd,
            shm_fd,
        })
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
            unsafe { core::slice::from_raw_parts_mut(self.shm_ptr as *mut u32, pixel_count) };
        Surface::from_raw(pixels, self.width, self.height, self.width)
    }

    /// Signal the compositor that drawing is complete and the surface is ready
    /// to be composited.
    pub fn commit(&self) {
        let msg = proto::commit(0);
        let mut buf = [0u8; MESSAGE_SIZE];
        // SAFETY: Commit is a 64-byte repr(C) message type.
        unsafe { proto::encode_msg(&msg, &mut buf) };
        let _ = sys::channel_send(self.channel_fd, &buf);
    }

    /// Poll for the next event from the compositor (non-blocking).
    ///
    /// Returns `None` if no event is pending.
    pub fn poll_event(&self) -> Option<Event> {
        if !sys::poll_fd_read(self.channel_fd) {
            return None;
        }

        let mut buf = [0u8; MESSAGE_SIZE];
        let n = sys::channel_recv(self.channel_fd, &mut buf).ok()?;
        if n < MESSAGE_SIZE {
            return None;
        }

        match proto::peek_opcode(&buf) {
            OP_KEYBOARD_INPUT => {
                // SAFETY: Opcode verified.
                let msg = unsafe { proto::cast_msg::<proto::KeyboardInput>(&buf) };
                Some(Event::Key {
                    character: msg.character,
                    keycode: msg.keycode,
                    pressed: msg.pressed != 0,
                })
            }
            OP_MOUSE_INPUT => {
                // SAFETY: Opcode verified.
                let msg = unsafe { proto::cast_msg::<proto::MouseInput>(&buf) };
                Some(Event::Mouse {
                    x: msg.x,
                    y: msg.y,
                    buttons: msg.buttons,
                    prev_buttons: msg.prev_buttons,
                })
            }
            OP_FOCUS_GAINED => Some(Event::FocusGained),
            OP_FOCUS_LOST => Some(Event::FocusLost),
            _ => None,
        }
    }

    /// Disconnect from the compositor, unmapping shared memory and closing fds.
    pub fn disconnect(self) {
        sys::mem_unmap(self.shm_ptr, self.shm_size);
        sys::close(self.shm_fd);
        sys::close(self.channel_fd);
    }
}
