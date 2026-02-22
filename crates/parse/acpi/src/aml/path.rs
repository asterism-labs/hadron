//! AML name segments and paths.
//!
//! ACPI names are composed of 4-byte segments. Paths are formed by chaining
//! segments together, with a maximum inline capacity of 16 segments sufficient
//! for all practical ACPI namespace depths.

/// A 4-byte AML name segment (e.g., `_SB_`, `PCI0`, `_HID`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NameSeg(pub [u8; 4]);

impl NameSeg {
    /// Create a `NameSeg` from a 4-byte slice.
    ///
    /// Returns `None` if the slice is shorter than 4 bytes.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        let mut seg = [0u8; 4];
        seg.copy_from_slice(&bytes[..4]);
        Some(Self(seg))
    }

    /// Returns the name as a UTF-8 string (ACPI names are always ASCII).
    #[must_use]
    pub fn as_str(&self) -> &str {
        // ACPI names are ASCII; fallback to empty on invalid UTF-8 (shouldn't happen).
        core::str::from_utf8(&self.0).unwrap_or("")
    }
}

impl core::fmt::Debug for NameSeg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "NameSeg(\"{}\")", self.as_str())
    }
}

impl core::fmt::Display for NameSeg {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Maximum number of segments in an inline AML path.
const MAX_PATH_DEPTH: usize = 16;

/// A fixed-capacity AML namespace path.
///
/// Stores up to [`MAX_PATH_DEPTH`] (16) segments inline, which is sufficient
/// for all practical ACPI namespace depths.
#[derive(Clone, Copy)]
pub struct AmlPath {
    segments: [NameSeg; MAX_PATH_DEPTH],
    len: u8,
}

impl AmlPath {
    /// The root path (`\`).
    pub const ROOT: Self = Self {
        segments: [NameSeg(*b"____"); MAX_PATH_DEPTH],
        len: 0,
    };

    /// Creates an empty path.
    #[must_use]
    pub const fn new() -> Self {
        Self::ROOT
    }

    /// Appends a name segment to the path.
    ///
    /// Returns `false` if the path is already at maximum capacity.
    pub fn push(&mut self, seg: NameSeg) -> bool {
        if (self.len as usize) >= MAX_PATH_DEPTH {
            return false;
        }
        self.segments[self.len as usize] = seg;
        self.len += 1;
        true
    }

    /// Removes and returns the last name segment from the path.
    pub fn pop(&mut self) -> Option<NameSeg> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        Some(self.segments[self.len as usize])
    }

    /// Returns the segments of this path.
    #[must_use]
    pub fn segments(&self) -> &[NameSeg] {
        &self.segments[..self.len as usize]
    }

    /// Returns the number of segments (depth) in this path.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.len as usize
    }
}

impl core::fmt::Debug for AmlPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "\\")?;
        for (i, seg) in self.segments().iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{seg}")?;
        }
        Ok(())
    }
}

impl core::fmt::Display for AmlPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "\\")?;
        for (i, seg) in self.segments().iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{seg}")?;
        }
        Ok(())
    }
}
