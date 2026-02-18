//! UEFI Status codes.
//!
//! This module defines the [`EfiStatus`] type, a newtype wrapper around `usize` that represents
//! UEFI status codes. Status codes are categorized into three groups:
//!
//! - **Success** (`0`): The operation completed successfully.
//! - **Warnings** (`1..HIGH_BIT`): The operation completed with a non-fatal condition.
//! - **Errors** (`HIGH_BIT..`): The operation failed.
//!
//! The high bit of the status code distinguishes errors from warnings/success.

use core::fmt;

/// The high bit of `usize`, used to distinguish error codes from warnings.
const ERROR_BIT: usize = 1 << (usize::BITS - 1);

/// A UEFI status code.
///
/// This is a transparent wrapper around `usize`, matching the UEFI `EFI_STATUS` type.
/// Use [`is_success`](EfiStatus::is_success), [`is_warning`](EfiStatus::is_warning), and
/// [`is_error`](EfiStatus::is_error) to classify the status, or
/// [`to_result`](EfiStatus::to_result) for ergonomic error handling.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EfiStatus(pub usize);

impl EfiStatus {
    // ── Success ──────────────────────────────────────────────────────

    /// The operation completed successfully.
    pub const SUCCESS: Self = Self(0);

    // ── Warning codes ────────────────────────────────────────────────

    /// The string contained characters that could not be rendered and were skipped.
    pub const WARN_UNKNOWN_GLYPH: Self = Self(1);
    /// The handle was closed, but the file was not deleted.
    pub const WARN_DELETE_FAILURE: Self = Self(2);
    /// The handle was closed, but the data to the file was not flushed properly.
    pub const WARN_WRITE_FAILURE: Self = Self(3);
    /// The resulting buffer was too small, and the data was truncated.
    pub const WARN_BUFFER_TOO_SMALL: Self = Self(4);
    /// The data has not been updated within the timeframe set by local policy.
    pub const WARN_STALE_DATA: Self = Self(5);
    /// The resulting buffer contains UEFI-compliant file system.
    pub const WARN_FILE_SYSTEM: Self = Self(6);
    /// The operation will be processed across a system reset.
    pub const WARN_RESET_REQUIRED: Self = Self(7);

    // ── Error codes ──────────────────────────────────────────────────

    /// The image failed to load.
    pub const LOAD_ERROR: Self = Self(ERROR_BIT | 1);
    /// A parameter was incorrect.
    pub const INVALID_PARAMETER: Self = Self(ERROR_BIT | 2);
    /// The operation is not supported.
    pub const UNSUPPORTED: Self = Self(ERROR_BIT | 3);
    /// The buffer was not the proper size for the request.
    pub const BAD_BUFFER_SIZE: Self = Self(ERROR_BIT | 4);
    /// The buffer is not large enough to hold the requested data.
    pub const BUFFER_TOO_SMALL: Self = Self(ERROR_BIT | 5);
    /// There is no data pending upon return.
    pub const NOT_READY: Self = Self(ERROR_BIT | 6);
    /// The physical device reported an error while attempting the operation.
    pub const DEVICE_ERROR: Self = Self(ERROR_BIT | 7);
    /// The device cannot be written to.
    pub const WRITE_PROTECTED: Self = Self(ERROR_BIT | 8);
    /// A resource has run out.
    pub const OUT_OF_RESOURCES: Self = Self(ERROR_BIT | 9);
    /// An inconsistency was detected on the file system.
    pub const VOLUME_CORRUPTED: Self = Self(ERROR_BIT | 0x0a);
    /// There is no more space on the file system.
    pub const VOLUME_FULL: Self = Self(ERROR_BIT | 0x0b);
    /// The device does not contain any medium to perform the operation.
    pub const NO_MEDIA: Self = Self(ERROR_BIT | 0x0c);
    /// The medium in the device has changed since the last access.
    pub const MEDIA_CHANGED: Self = Self(ERROR_BIT | 0x0d);
    /// The item was not found.
    pub const NOT_FOUND: Self = Self(ERROR_BIT | 0x0e);
    /// Access was denied.
    pub const ACCESS_DENIED: Self = Self(ERROR_BIT | 0x0f);
    /// The server was not found or did not respond to the request.
    pub const NO_RESPONSE: Self = Self(ERROR_BIT | 0x10);
    /// A mapping to a device does not exist.
    pub const NO_MAPPING: Self = Self(ERROR_BIT | 0x11);
    /// The timeout time expired.
    pub const TIMEOUT: Self = Self(ERROR_BIT | 0x12);
    /// The protocol has not been started.
    pub const NOT_STARTED: Self = Self(ERROR_BIT | 0x13);
    /// The protocol has already been started.
    pub const ALREADY_STARTED: Self = Self(ERROR_BIT | 0x14);
    /// The operation was aborted.
    pub const ABORTED: Self = Self(ERROR_BIT | 0x15);
    /// An ICMP error occurred during the network operation.
    pub const ICMP_ERROR: Self = Self(ERROR_BIT | 0x16);
    /// A TFTP error occurred during the network operation.
    pub const TFTP_ERROR: Self = Self(ERROR_BIT | 0x17);
    /// A protocol error occurred during the network operation.
    pub const PROTOCOL_ERROR: Self = Self(ERROR_BIT | 0x18);
    /// The function encountered an internal version that was incompatible.
    pub const INCOMPATIBLE_VERSION: Self = Self(ERROR_BIT | 0x19);
    /// The function was not performed due to a security violation.
    pub const SECURITY_VIOLATION: Self = Self(ERROR_BIT | 0x1a);
    /// A CRC error was detected.
    pub const CRC_ERROR: Self = Self(ERROR_BIT | 0x1b);
    /// Beginning or end of media was reached.
    pub const END_OF_MEDIA: Self = Self(ERROR_BIT | 0x1c);
    /// The end of the file was reached.
    pub const END_OF_FILE: Self = Self(ERROR_BIT | 0x1f);
    /// The language specified was invalid.
    pub const INVALID_LANGUAGE: Self = Self(ERROR_BIT | 0x20);
    /// The security status of the data is unknown or compromised.
    pub const COMPROMISED_DATA: Self = Self(ERROR_BIT | 0x21);
    /// There is an address conflict during the IP address configuration.
    pub const IP_ADDRESS_CONFLICT: Self = Self(ERROR_BIT | 0x22);
    /// An HTTP error occurred during the network operation.
    pub const HTTP_ERROR: Self = Self(ERROR_BIT | 0x23);

    /// Returns `true` if this status code indicates success.
    #[inline]
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 == 0
    }

    /// Returns `true` if this status code indicates an error (high bit set).
    #[inline]
    #[must_use]
    pub const fn is_error(self) -> bool {
        self.0 & ERROR_BIT != 0
    }

    /// Returns `true` if this status code indicates a warning (non-zero, high bit clear).
    #[inline]
    #[must_use]
    pub const fn is_warning(self) -> bool {
        !self.is_success() && !self.is_error()
    }

    /// Converts this status code to a `Result`.
    ///
    /// Returns `Ok(())` if the status is success or a warning, `Err(self)` if it is an error.
    ///
    /// # Errors
    ///
    /// Returns `Err(EfiStatus)` if the status code indicates an error (high bit set).
    #[inline]
    pub const fn to_result(self) -> Result<(), Self> {
        if self.is_error() { Err(self) } else { Ok(()) }
    }

    /// Returns a human-readable name for the status code, if known.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self {
            Self::SUCCESS => Some("EFI_SUCCESS"),
            Self::WARN_UNKNOWN_GLYPH => Some("EFI_WARN_UNKNOWN_GLYPH"),
            Self::WARN_DELETE_FAILURE => Some("EFI_WARN_DELETE_FAILURE"),
            Self::WARN_WRITE_FAILURE => Some("EFI_WARN_WRITE_FAILURE"),
            Self::WARN_BUFFER_TOO_SMALL => Some("EFI_WARN_BUFFER_TOO_SMALL"),
            Self::WARN_STALE_DATA => Some("EFI_WARN_STALE_DATA"),
            Self::WARN_FILE_SYSTEM => Some("EFI_WARN_FILE_SYSTEM"),
            Self::WARN_RESET_REQUIRED => Some("EFI_WARN_RESET_REQUIRED"),
            Self::LOAD_ERROR => Some("EFI_LOAD_ERROR"),
            Self::INVALID_PARAMETER => Some("EFI_INVALID_PARAMETER"),
            Self::UNSUPPORTED => Some("EFI_UNSUPPORTED"),
            Self::BAD_BUFFER_SIZE => Some("EFI_BAD_BUFFER_SIZE"),
            Self::BUFFER_TOO_SMALL => Some("EFI_BUFFER_TOO_SMALL"),
            Self::NOT_READY => Some("EFI_NOT_READY"),
            Self::DEVICE_ERROR => Some("EFI_DEVICE_ERROR"),
            Self::WRITE_PROTECTED => Some("EFI_WRITE_PROTECTED"),
            Self::OUT_OF_RESOURCES => Some("EFI_OUT_OF_RESOURCES"),
            Self::VOLUME_CORRUPTED => Some("EFI_VOLUME_CORRUPTED"),
            Self::VOLUME_FULL => Some("EFI_VOLUME_FULL"),
            Self::NO_MEDIA => Some("EFI_NO_MEDIA"),
            Self::MEDIA_CHANGED => Some("EFI_MEDIA_CHANGED"),
            Self::NOT_FOUND => Some("EFI_NOT_FOUND"),
            Self::ACCESS_DENIED => Some("EFI_ACCESS_DENIED"),
            Self::NO_RESPONSE => Some("EFI_NO_RESPONSE"),
            Self::NO_MAPPING => Some("EFI_NO_MAPPING"),
            Self::TIMEOUT => Some("EFI_TIMEOUT"),
            Self::NOT_STARTED => Some("EFI_NOT_STARTED"),
            Self::ALREADY_STARTED => Some("EFI_ALREADY_STARTED"),
            Self::ABORTED => Some("EFI_ABORTED"),
            Self::ICMP_ERROR => Some("EFI_ICMP_ERROR"),
            Self::TFTP_ERROR => Some("EFI_TFTP_ERROR"),
            Self::PROTOCOL_ERROR => Some("EFI_PROTOCOL_ERROR"),
            Self::INCOMPATIBLE_VERSION => Some("EFI_INCOMPATIBLE_VERSION"),
            Self::SECURITY_VIOLATION => Some("EFI_SECURITY_VIOLATION"),
            Self::CRC_ERROR => Some("EFI_CRC_ERROR"),
            Self::END_OF_MEDIA => Some("EFI_END_OF_MEDIA"),
            Self::END_OF_FILE => Some("EFI_END_OF_FILE"),
            Self::INVALID_LANGUAGE => Some("EFI_INVALID_LANGUAGE"),
            Self::COMPROMISED_DATA => Some("EFI_COMPROMISED_DATA"),
            Self::IP_ADDRESS_CONFLICT => Some("EFI_IP_ADDRESS_CONFLICT"),
            Self::HTTP_ERROR => Some("EFI_HTTP_ERROR"),
            _ => None,
        }
    }
}

impl fmt::Debug for EfiStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "EfiStatus({name})"),
            None => write!(f, "EfiStatus({:#x})", self.0),
        }
    }
}

impl fmt::Display for EfiStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None if self.is_error() => {
                write!(f, "Unknown error ({:#x})", self.0 & !ERROR_BIT)
            }
            None => write!(f, "Unknown warning ({})", self.0),
        }
    }
}

// ── Compile-time layout assertions ──────────────────────────────────

#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<EfiStatus>() == 8);
