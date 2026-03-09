//! Wayland protocol constants shared between compositor and client.
//!
//! Defines interface names, opcodes for requests and events, pixel format
//! constants, and well-known global names.

// -- Interface names ---------------------------------------------------------

/// `wl_compositor` interface name.
pub const WL_COMPOSITOR: &[u8] = b"wl_compositor";
/// `wl_shm` interface name.
pub const WL_SHM: &[u8] = b"wl_shm";
/// `xdg_wm_base` interface name.
pub const XDG_WM_BASE: &[u8] = b"xdg_wm_base";

// -- Well-known global names (registry bind targets) -------------------------

/// Global name for `wl_compositor`.
pub const GLOBAL_COMPOSITOR: u32 = 1;
/// Global name for `wl_shm`.
pub const GLOBAL_SHM: u32 = 2;
/// Global name for `xdg_wm_base`.
pub const GLOBAL_XDG_WM_BASE: u32 = 3;

// -- Interface versions ------------------------------------------------------

/// `wl_compositor` version advertised by the compositor.
pub const WL_COMPOSITOR_VERSION: u32 = 4;
/// `wl_shm` version advertised by the compositor.
pub const WL_SHM_VERSION: u32 = 1;
/// `xdg_wm_base` version advertised by the compositor.
pub const XDG_WM_BASE_VERSION: u32 = 2;

// -- wl_display opcodes (object 1) -------------------------------------------

/// Client request: `wl_display.get_registry(new_id)` — opcode 1.
pub const WL_DISPLAY_GET_REGISTRY: u16 = 1;

// -- wl_registry opcodes (client-allocated) ----------------------------------

/// Client request: `wl_registry.bind(name, interface, version, new_id)` — opcode 0.
pub const WL_REGISTRY_BIND: u16 = 0;
/// Server event: `wl_registry.global(name, interface, version)` — opcode 0.
pub const WL_REGISTRY_GLOBAL: u16 = 0;

// -- wl_compositor opcodes ---------------------------------------------------

/// Client request: `wl_compositor.create_surface(new_id)` — opcode 0.
pub const WL_COMPOSITOR_CREATE_SURFACE: u16 = 0;

// -- wl_surface opcodes ------------------------------------------------------

/// Client request: `wl_surface.destroy()` — opcode 0.
pub const WL_SURFACE_DESTROY: u16 = 0;
/// Client request: `wl_surface.attach(buffer, x, y)` — opcode 1.
pub const WL_SURFACE_ATTACH: u16 = 1;
/// Client request: `wl_surface.frame(callback)` — opcode 3.
pub const WL_SURFACE_FRAME: u16 = 3;
/// Client request: `wl_surface.commit()` — opcode 6.
pub const WL_SURFACE_COMMIT: u16 = 6;

// -- wl_shm opcodes ----------------------------------------------------------

/// Client request: `wl_shm.create_pool(new_id, fd, size)` — opcode 0.
pub const WL_SHM_CREATE_POOL: u16 = 0;
/// Server event: `wl_shm.format(format)` — opcode 0.
pub const WL_SHM_FORMAT_EVENT: u16 = 0;

// -- wl_shm_pool opcodes ----------------------------------------------------

/// Client request: `wl_shm_pool.create_buffer(new_id, offset, w, h, stride, format)` — opcode 0.
pub const WL_SHM_POOL_CREATE_BUFFER: u16 = 0;
/// Client request: `wl_shm_pool.destroy()` — opcode 2.
pub const WL_SHM_POOL_DESTROY: u16 = 2;

// -- wl_buffer opcodes -------------------------------------------------------

/// Client request: `wl_buffer.destroy()` — opcode 0.
pub const WL_BUFFER_DESTROY: u16 = 0;
/// Server event: `wl_buffer.release()` — opcode 0.
pub const WL_BUFFER_RELEASE: u16 = 0;

// -- xdg_wm_base opcodes ----------------------------------------------------

/// Client request: `xdg_wm_base.pong(serial)` — opcode 3.
pub const XDG_WM_BASE_PONG: u16 = 3;
/// Client request: `xdg_wm_base.get_xdg_surface(new_id, surface)` — opcode 2.
pub const XDG_WM_BASE_GET_XDG_SURFACE: u16 = 2;
/// Server event: `xdg_wm_base.ping(serial)` — opcode 0.
pub const XDG_WM_BASE_PING: u16 = 0;

// -- xdg_surface opcodes ----------------------------------------------------

/// Client request: `xdg_surface.get_toplevel(new_id)` — opcode 1.
pub const XDG_SURFACE_GET_TOPLEVEL: u16 = 1;
/// Client request: `xdg_surface.ack_configure(serial)` — opcode 4.
pub const XDG_SURFACE_ACK_CONFIGURE: u16 = 4;
/// Server event: `xdg_surface.configure(serial)` — opcode 0.
pub const XDG_SURFACE_CONFIGURE: u16 = 0;

// -- xdg_toplevel opcodes ----------------------------------------------------

/// Server event: `xdg_toplevel.configure(width, height, states)` — opcode 0.
pub const XDG_TOPLEVEL_CONFIGURE: u16 = 0;

// -- SHM pixel formats -------------------------------------------------------

/// 32-bit ARGB, little-endian.
pub const WL_SHM_FORMAT_ARGB8888: u32 = 0;
/// 32-bit XRGB (opaque), little-endian.
pub const WL_SHM_FORMAT_XRGB8888: u32 = 1;

// -- xdg_toplevel states -----------------------------------------------------

/// Window has keyboard focus.
pub const XDG_TOPLEVEL_STATE_ACTIVATED: u32 = 4;

// -- Socket path -------------------------------------------------------------

/// Default Wayland socket path.
pub const WAYLAND_SOCKET_PATH: &[u8] = b"/run/wayland-0";
