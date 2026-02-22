//! Early device drivers and driver infrastructure.

#[cfg(target_os = "none")]
pub mod device_registry;
pub mod early_console;
pub mod early_fb;
#[cfg(target_os = "none")]
pub mod fbcon;
pub mod font_console;
pub mod irq;
#[cfg(target_os = "none")]
pub mod registry;
