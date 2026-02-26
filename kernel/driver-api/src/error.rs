//! Driver error types.

use core::fmt;

/// Errors that can occur during driver operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverError {
    /// The hardware device was not found or did not respond.
    DeviceNotFound,
    /// Driver initialization failed.
    InitFailed,
    /// A hardware operation timed out.
    Timeout,
    /// The requested operation is not supported by this driver.
    Unsupported,
    /// An I/O error occurred during a hardware operation.
    IoError,
    /// The driver is not in a valid state for this operation.
    InvalidState,
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound => f.write_str("device not found"),
            Self::InitFailed => f.write_str("driver initialization failed"),
            Self::Timeout => f.write_str("hardware operation timed out"),
            Self::Unsupported => f.write_str("operation not supported"),
            Self::IoError => f.write_str("I/O error"),
            Self::InvalidState => f.write_str("invalid driver state"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_all_variants() {
        assert_eq!(
            format!("{}", DriverError::DeviceNotFound),
            "device not found"
        );
        assert_eq!(
            format!("{}", DriverError::InitFailed),
            "driver initialization failed"
        );
        assert_eq!(
            format!("{}", DriverError::Timeout),
            "hardware operation timed out"
        );
        assert_eq!(
            format!("{}", DriverError::Unsupported),
            "operation not supported"
        );
        assert_eq!(format!("{}", DriverError::IoError), "I/O error");
        assert_eq!(
            format!("{}", DriverError::InvalidState),
            "invalid driver state"
        );
    }

    #[test]
    fn error_equality() {
        assert_eq!(DriverError::DeviceNotFound, DriverError::DeviceNotFound);
        assert_ne!(DriverError::DeviceNotFound, DriverError::InitFailed);
    }
}
