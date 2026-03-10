//! Serial port configuration types for QEMU.

use std::path::PathBuf;

/// Configuration for a single QEMU serial port.
#[derive(Debug, Clone)]
pub enum SerialConfig {
    /// Route to process stdio (COM1 console).
    Stdio,
    /// Route to a file (COM2 logging).
    File(PathBuf),
    /// Route to a Unix socket.
    Socket(PathBuf),
    /// Disable this serial port.
    None,
}

impl SerialConfig {
    /// Convert to QEMU `-serial` argument value.
    pub(crate) fn to_qemu_arg(&self) -> String {
        match self {
            Self::Stdio => "stdio".to_string(),
            Self::File(path) => format!("file:{}", path.display()),
            Self::Socket(path) => format!("unix:{},server=on,wait=off", path.display()),
            Self::None => "none".to_string(),
        }
    }
}
