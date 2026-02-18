//! Input device drivers.

pub mod i8042;
#[cfg(target_arch = "x86_64")]
pub mod keyboard_async;
#[cfg(target_arch = "x86_64")]
pub mod mouse_async;
