//! Input device drivers.

#[cfg(hadron_driver_i8042)]
pub mod i8042;
#[cfg(hadron_driver_i8042)]
pub mod keyboard_async;
#[cfg(hadron_driver_i8042)]
pub mod mouse_async;
