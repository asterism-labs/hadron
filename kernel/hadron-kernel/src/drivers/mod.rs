//! Early device drivers and driver infrastructure.

#[cfg(target_os = "none")]
pub mod device_registry;
pub mod early_console;
pub mod early_fb;
pub mod irq;
pub mod font_console;
#[cfg(target_os = "none")]
pub mod registry;
