//! Binary format registry.
//!
//! A trait-based loader system that probes raw binary data, identifies the
//! executable format, and returns a uniform [`ExecImage`] for the process
//! creator. Handlers are tried in registration order; the first whose
//! [`BinaryFormat::probe`] returns `true` wins.

pub mod elf;
pub mod reloc;
pub mod script;

use core::fmt;

use noalloc::vec::ArrayVec;

/// Maximum number of loadable segments an [`ExecImage`] can hold.
const MAX_SEGMENTS: usize = 16;

/// Permission flags for a loadable segment.
pub struct SegmentFlags {
    /// The segment is writable.
    pub writable: bool,
    /// The segment is executable.
    pub executable: bool,
}

/// A single loadable segment extracted from a binary.
pub struct ExecSegment<'a> {
    /// Virtual address where this segment is loaded.
    pub vaddr: u64,
    /// File content (zero-copy borrow from input). Remainder up to `memsz`
    /// is zero-filled by the mapper.
    pub data: &'a [u8],
    /// Total size in memory (>= `data.len()`).
    pub memsz: u64,
    /// Permission flags.
    pub flags: SegmentFlags,
}

/// A parsed executable image ready for mapping into an address space.
pub struct ExecImage<'a> {
    /// Virtual address of the entry point.
    pub entry_point: u64,
    /// Load base address: 0 for `ET_EXEC`, chosen base for `ET_DYN`.
    pub base_addr: u64,
    /// Whether the image requires post-mapping relocation.
    pub needs_relocation: bool,
    /// Raw ELF data for the relocation pass (only set when `needs_relocation` is true).
    pub elf_data: Option<&'a [u8]>,
    /// Loadable segments.
    segments: ArrayVec<ExecSegment<'a>, MAX_SEGMENTS>,
}

impl<'a> ExecImage<'a> {
    /// Returns a slice over the loadable segments.
    #[must_use]
    pub fn segments(&self) -> &[ExecSegment<'a>] {
        self.segments.as_slice()
    }
}

/// Errors that can occur while loading a binary.
#[derive(Debug)]
pub enum BinaryError {
    /// No registered handler recognised the format.
    UnrecognizedFormat,
    /// The handler recognised the format but parsing failed.
    ParseError(&'static str),
    /// Too many loadable segments for the fixed-capacity buffer.
    TooManySegments,
    /// The format is recognised but not yet implemented.
    Unimplemented(&'static str),
    /// A relocation failed to apply.
    RelocError(hadron_elf::RelocError),
}

impl fmt::Display for BinaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BinaryError::UnrecognizedFormat => write!(f, "unrecognized binary format"),
            BinaryError::ParseError(msg) => write!(f, "binary parse error: {msg}"),
            BinaryError::TooManySegments => write!(f, "too many loadable segments"),
            BinaryError::Unimplemented(what) => write!(f, "unimplemented format: {what}"),
            BinaryError::RelocError(e) => write!(f, "relocation error: {e}"),
        }
    }
}

/// A handler that can probe and load a particular binary format.
///
/// Object-safe: the lifetime `'a` is late-bound (on the method, not the
/// trait). `Sync` is required because the registry is a `static` slice.
pub trait BinaryFormat: Sync {
    /// Human-readable name of this format (e.g. `"ELF"`).
    fn name(&self) -> &'static str;

    /// Returns `true` if `data` begins with this format's magic bytes.
    fn probe(&self, data: &[u8]) -> bool;

    /// Parses `data` and returns an [`ExecImage`] with zero-copy segment
    /// references into the input slice.
    ///
    /// # Errors
    ///
    /// Returns [`BinaryError`] if parsing fails.
    fn load<'a>(&self, data: &'a [u8]) -> Result<ExecImage<'a>, BinaryError>;
}

/// Registered binary format handlers, tried in order.
static BINARY_FORMATS: &[&dyn BinaryFormat] = &[&elf::ElfHandler, &script::ScriptHandler];

/// Probes `data` against all registered formats and loads the first match.
///
/// # Errors
///
/// Returns [`BinaryError::UnrecognizedFormat`] if no handler matches, or
/// a handler-specific error if parsing fails.
pub fn load_binary(data: &[u8]) -> Result<ExecImage<'_>, BinaryError> {
    for handler in BINARY_FORMATS {
        if handler.probe(data) {
            return handler.load(data);
        }
    }
    Err(BinaryError::UnrecognizedFormat)
}
