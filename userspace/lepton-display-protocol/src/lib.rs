//! Display protocol for compositor-client communication.
//!
//! All messages are fixed 64-byte `#[repr(C)]` structs sent over a channel.
//! The first two bytes of every message are a little-endian opcode that
//! identifies the message type.

#![no_std]

/// Size of every wire message in bytes.
pub const MESSAGE_SIZE: usize = 64;

// ── Opcodes ──────────────────────────────────────────────────────────

/// Compositor tells client its surface dimensions.
pub const OP_CONFIGURE: u16 = 0x0101;
/// Compositor forwards a keyboard event.
pub const OP_KEYBOARD_INPUT: u16 = 0x0102;
/// Compositor notifies client it gained focus.
pub const OP_FOCUS_GAINED: u16 = 0x0103;
/// Compositor notifies client it lost focus.
pub const OP_FOCUS_LOST: u16 = 0x0104;
/// Client tells compositor it finished drawing.
pub const OP_COMMIT: u16 = 0x0005;

// ── Common header ────────────────────────────────────────────────────

/// Header present at the start of every message.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MsgHeader {
    /// Message opcode (one of the `OP_*` constants).
    pub opcode: u16,
    /// Reserved padding.
    pub _pad: u16,
    /// Surface identifier (0 for the single surface in this protocol).
    pub surface_id: u32,
}

// ── Compositor → Client ──────────────────────────────────────────────

/// Initial configuration sent to a client after spawn.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Configure {
    /// Message header (opcode = `OP_CONFIGURE`).
    pub header: MsgHeader,
    /// Surface width in pixels.
    pub width: u32,
    /// Surface height in pixels.
    pub height: u32,
    /// Reserved for future use.
    pub _reserved: [u8; 48],
}

/// Keyboard input event forwarded by the compositor.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct KeyboardInput {
    /// Message header (opcode = `OP_KEYBOARD_INPUT`).
    pub header: MsgHeader,
    /// Raw scancode / keycode.
    pub keycode: u32,
    /// 1 = pressed, 0 = released.
    pub pressed: u32,
    /// Modifier flags (reserved, currently 0).
    pub modifiers: u32,
    /// ASCII character (0 if non-printable).
    pub character: u8,
    /// Reserved padding.
    pub _reserved: [u8; 43],
}

/// Notification that this client's surface gained keyboard focus.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FocusGained {
    /// Message header (opcode = `OP_FOCUS_GAINED`).
    pub header: MsgHeader,
    /// Reserved for future use.
    pub _reserved: [u8; 56],
}

/// Notification that this client's surface lost keyboard focus.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FocusLost {
    /// Message header (opcode = `OP_FOCUS_LOST`).
    pub header: MsgHeader,
    /// Reserved for future use.
    pub _reserved: [u8; 56],
}

// ── Client → Compositor ──────────────────────────────────────────────

/// Client signals that it has finished drawing and the surface is ready.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Commit {
    /// Message header (opcode = `OP_COMMIT`).
    pub header: MsgHeader,
    /// Reserved for future use.
    pub _reserved: [u8; 56],
}

// ── Compile-time size assertions ─────────────────────────────────────

const _: () = assert!(size_of::<Configure>() == MESSAGE_SIZE);
const _: () = assert!(size_of::<KeyboardInput>() == MESSAGE_SIZE);
const _: () = assert!(size_of::<FocusGained>() == MESSAGE_SIZE);
const _: () = assert!(size_of::<FocusLost>() == MESSAGE_SIZE);
const _: () = assert!(size_of::<Commit>() == MESSAGE_SIZE);

// ── Wire helpers ─────────────────────────────────────────────────────

/// Read the header from a raw message buffer.
///
/// # Safety
///
/// `buf` must contain at least `size_of::<MsgHeader>()` valid bytes.
pub fn peek_opcode(buf: &[u8; MESSAGE_SIZE]) -> u16 {
    u16::from_le_bytes([buf[0], buf[1]])
}

/// Interpret a raw message buffer as a typed message reference.
///
/// # Safety
///
/// Caller must verify the opcode matches `T` before calling. `T` must be
/// one of the 64-byte `#[repr(C)]` message types defined in this crate.
pub unsafe fn cast_msg<T: Copy>(buf: &[u8; MESSAGE_SIZE]) -> &T {
    debug_assert!(size_of::<T>() == MESSAGE_SIZE);
    // SAFETY: T is repr(C), same size as buf, and caller verified opcode.
    unsafe { &*buf.as_ptr().cast::<T>() }
}

/// Encode a typed message into a raw 64-byte buffer.
///
/// # Safety
///
/// `T` must be one of the 64-byte `#[repr(C)]` message types defined in
/// this crate.
pub unsafe fn encode_msg<T: Copy>(msg: &T, buf: &mut [u8; MESSAGE_SIZE]) {
    debug_assert!(size_of::<T>() == MESSAGE_SIZE);
    // SAFETY: T is repr(C), same size as buf.
    unsafe {
        core::ptr::copy_nonoverlapping(
            msg as *const T as *const u8,
            buf.as_mut_ptr(),
            MESSAGE_SIZE,
        );
    }
}

// ── Builder helpers ──────────────────────────────────────────────────

/// Create a `Configure` message.
pub fn configure(surface_id: u32, width: u32, height: u32) -> Configure {
    Configure {
        header: MsgHeader {
            opcode: OP_CONFIGURE,
            _pad: 0,
            surface_id,
        },
        width,
        height,
        _reserved: [0; 48],
    }
}

/// Create a `KeyboardInput` message.
pub fn keyboard_input(
    surface_id: u32,
    keycode: u32,
    pressed: bool,
    character: u8,
) -> KeyboardInput {
    KeyboardInput {
        header: MsgHeader {
            opcode: OP_KEYBOARD_INPUT,
            _pad: 0,
            surface_id,
        },
        keycode,
        pressed: u32::from(pressed),
        modifiers: 0,
        character,
        _reserved: [0; 43],
    }
}

/// Create a `FocusGained` message.
pub fn focus_gained(surface_id: u32) -> FocusGained {
    FocusGained {
        header: MsgHeader {
            opcode: OP_FOCUS_GAINED,
            _pad: 0,
            surface_id,
        },
        _reserved: [0; 56],
    }
}

/// Create a `FocusLost` message.
pub fn focus_lost(surface_id: u32) -> FocusLost {
    FocusLost {
        header: MsgHeader {
            opcode: OP_FOCUS_LOST,
            _pad: 0,
            surface_id,
        },
        _reserved: [0; 56],
    }
}

/// Create a `Commit` message.
pub fn commit(surface_id: u32) -> Commit {
    Commit {
        header: MsgHeader {
            opcode: OP_COMMIT,
            _pad: 0,
            surface_id,
        },
        _reserved: [0; 56],
    }
}
