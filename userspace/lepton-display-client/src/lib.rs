//! Display client library for compositor-managed applications.
//!
//! Clients receive a channel endpoint on fd 3 and a shared-memory buffer on
//! fd 4 from the compositor at spawn time. [`Display::connect`] waits for the
//! initial [`Configure`](lepton_display_protocol::Configure) message and maps
//! the shared buffer. The client draws into the [`Surface`] and calls
//! [`Display::commit`] to signal the compositor.

#![no_std]

extern crate alloc;

use lepton_display_protocol::{
    self as proto, MESSAGE_SIZE, OP_CONFIGURE, OP_FOCUS_GAINED, OP_FOCUS_LOST, OP_KEYBOARD_INPUT,
    OP_MOUSE_INPUT,
};
use lepton_gfx::Surface;
use lepton_syslib::sys;

/// Channel endpoint fd inherited from the compositor.
const CHANNEL_FD: usize = 3;
/// Shared-memory fd inherited from the compositor.
const SHM_FD: usize = 4;

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
    /// Connect to the compositor by reading the initial `Configure` message
    /// on fd 3 and mapping the shared buffer from fd 4.
    ///
    /// Returns `None` if the channel read fails or the first message is not
    /// a `Configure`.
    pub fn connect() -> Option<Self> {
        let mut buf = [0u8; MESSAGE_SIZE];
        let n = sys::channel_recv(CHANNEL_FD, &mut buf).ok()?;
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
        let shm_ptr = sys::mem_map_shared(SHM_FD, shm_size)?;

        Some(Display {
            width,
            height,
            shm_ptr,
            shm_size,
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
        let _ = sys::channel_send(CHANNEL_FD, &buf);
    }

    /// Poll for the next event from the compositor (non-blocking).
    ///
    /// Returns `None` if no event is pending.
    pub fn poll_event(&self) -> Option<Event> {
        if !sys::poll_fd_read(CHANNEL_FD) {
            return None;
        }

        let mut buf = [0u8; MESSAGE_SIZE];
        let n = sys::channel_recv(CHANNEL_FD, &mut buf).ok()?;
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
        sys::close(SHM_FD);
        sys::close(CHANNEL_FD);
    }
}
