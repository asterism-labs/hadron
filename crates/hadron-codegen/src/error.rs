//! Error types for the code generator.

use std::fmt;
use std::io;

/// Errors that can occur during code generation.
#[derive(Debug)]
pub enum CodegenError {
    /// I/O error reading a font file.
    FontIo(io::Error),
    /// Failed to parse or load a font.
    FontLoad(String),
    /// A requested codepoint is invalid.
    InvalidCodepoint(u32),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FontIo(e) => write!(f, "font I/O error: {e}"),
            Self::FontLoad(msg) => write!(f, "font load error: {msg}"),
            Self::InvalidCodepoint(cp) => write!(f, "invalid codepoint: U+{cp:04X}"),
        }
    }
}

impl std::error::Error for CodegenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::FontIo(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for CodegenError {
    fn from(e: io::Error) -> Self {
        Self::FontIo(e)
    }
}
