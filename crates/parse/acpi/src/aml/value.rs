//! AML data object values and error types.

use crate::AcpiError;

/// Maximum length of an inline string in an [`AmlValue`].
const INLINE_STRING_CAP: usize = 16;

/// An inline string with a fixed maximum capacity.
///
/// Sufficient for `_HID` strings like `"ACPI0004"` and EISA IDs.
#[derive(Clone, Copy)]
pub struct InlineString {
    buf: [u8; INLINE_STRING_CAP],
    len: u8,
}

impl InlineString {
    /// Creates a new `InlineString` from a byte slice.
    ///
    /// Truncates to [`INLINE_STRING_CAP`] bytes if the input is longer.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let len = bytes.len().min(INLINE_STRING_CAP);
        let mut buf = [0u8; INLINE_STRING_CAP];
        buf[..len].copy_from_slice(&bytes[..len]);
        Self {
            buf,
            len: len as u8,
        }
    }

    /// Returns the string as a UTF-8 `&str`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}

impl core::fmt::Debug for InlineString {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "\"{}\"", self.as_str())
    }
}

/// A compressed EISA/PnP device identifier.
///
/// EISA IDs are stored as 32-bit compressed values in AML bytecode
/// (via the `EisaId()` macro in ASL). The 3-letter manufacturer code
/// is packed into the upper 16 bits and the product ID into the lower 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EisaId {
    /// The raw 32-bit EISA ID value (byte-swapped from AML encoding).
    pub raw: u32,
}

impl EisaId {
    /// Decodes the EISA ID into a 7-character ASCII string (e.g., `"PNP0A03"`).
    #[must_use]
    pub fn decode(&self) -> [u8; 7] {
        // The EISA ID is stored big-endian in AML. After byte-swapping to
        // native order, the encoding is:
        //   Bits 30-26: first char - 'A' + 1
        //   Bits 25-21: second char - 'A' + 1
        //   Bits 20-16: third char - 'A' + 1
        //   Bits 15-0:  product ID as 4 hex digits
        let swapped = self.raw.swap_bytes();
        let c1 = (((swapped >> 26) & 0x1F) as u8) + b'@';
        let c2 = (((swapped >> 21) & 0x1F) as u8) + b'@';
        let c3 = (((swapped >> 16) & 0x1F) as u8) + b'@';
        let product = swapped as u16;

        let hex_digit = |nibble: u8| -> u8 {
            if nibble < 10 {
                b'0' + nibble
            } else {
                b'A' + nibble - 10
            }
        };

        [
            c1,
            c2,
            c3,
            hex_digit((product >> 12) as u8 & 0xF),
            hex_digit((product >> 8) as u8 & 0xF),
            hex_digit((product >> 4) as u8 & 0xF),
            hex_digit(product as u8 & 0xF),
        ]
    }
}

/// A resolved AML data object value.
///
/// Only a subset of AML values are fully resolved during a single-pass walk.
/// Complex objects (buffers, packages, method results) are represented as
/// [`AmlValue::Unresolved`].
#[derive(Debug, Clone, Copy)]
pub enum AmlValue {
    /// An integer constant (Zero, One, Ones, ByteConst, WordConst,
    /// DWordConst, QWordConst).
    Integer(u64),
    /// A compressed EISA/PnP device identifier.
    EisaId(EisaId),
    /// A short inline string (e.g., `_HID` string values like `"ACPI0004"`).
    String(InlineString),
    /// A value that cannot be resolved in a single pass (Buffer, Package,
    /// method call, etc.).
    Unresolved,
}

/// Errors specific to AML bytecode parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmlError {
    /// The AML stream ended unexpectedly.
    UnexpectedEnd,
    /// A PkgLength encoding was invalid.
    InvalidPkgLength,
    /// An [`AmlPath`](super::path::AmlPath) exceeded its maximum depth.
    PathOverflow,
    /// The AML bytecode contained an invalid or unsupported construct.
    InvalidAml,
    /// An underlying ACPI table error.
    AcpiError(AcpiError),
}

impl From<AcpiError> for AmlError {
    fn from(e: AcpiError) -> Self {
        Self::AcpiError(e)
    }
}
