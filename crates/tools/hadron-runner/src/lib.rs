//! Hadron runner — ISO creation, QEMU management, and QMP scripting.
//!
//! Replaces `cargo-image-runner` with a purpose-built library that provides:
//! - Direct ISO creation via `hadris-iso`
//! - Limine bootloader binary caching
//! - QEMU process management with configurable serial ports
//! - QMP protocol client for keyboard/mouse/screenshot scripting

pub mod iso;
pub mod limine;
pub mod qemu;
pub mod qmp;
pub mod serial;

pub use iso::IsoBuilder;
pub use limine::LimineCache;
pub use qemu::{DisplayConfig, QemuConfig, QemuExit, RunningQemu, TestConfig};
pub use qmp::QmpClient;
pub mod scripting;
pub use serial::{SerialConfig, SerialPty};
