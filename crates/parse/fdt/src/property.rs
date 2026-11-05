//! FDT property types and typed accessors.

/// A single property from an FDT node.
#[derive(Debug, Clone, Copy)]
pub struct FdtProperty<'a> {
    name: &'a str,
    data: &'a [u8],
}

impl<'a> FdtProperty<'a> {
    /// Creates a new property with the given name and raw data.
    pub(crate) fn new(name: &'a str, data: &'a [u8]) -> Self {
        Self { name, data }
    }

    /// Returns the property name.
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Returns the raw property data.
    #[must_use]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Returns the length of the property data in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if the property data is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Interprets the property as a big-endian `u32`.
    #[must_use]
    pub fn as_u32(&self) -> Option<u32> {
        if self.data.len() < 4 {
            return None;
        }
        let bytes: [u8; 4] = self.data[..4].try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    }

    /// Interprets the property as a big-endian `u64`.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        if self.data.len() < 8 {
            return None;
        }
        let bytes: [u8; 8] = self.data[..8].try_into().ok()?;
        Some(u64::from_be_bytes(bytes))
    }

    /// Interprets the property as a null-terminated UTF-8 string.
    #[must_use]
    pub fn as_str(&self) -> Option<&'a str> {
        // Strip the trailing null if present.
        let bytes = if self.data.last() == Some(&0) {
            &self.data[..self.data.len() - 1]
        } else {
            self.data
        };
        core::str::from_utf8(bytes).ok()
    }

    /// Returns an iterator over a null-separated string list.
    #[must_use]
    pub fn as_str_list(&self) -> StrListIter<'a> {
        StrListIter { data: self.data }
    }
}

/// Iterator over a null-separated string list property.
///
/// FDT `compatible` and similar properties store multiple null-terminated
/// strings back-to-back. This iterator yields each one.
pub struct StrListIter<'a> {
    data: &'a [u8],
}

impl<'a> Iterator for StrListIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.data.is_empty() {
            return None;
        }

        // Find the next null terminator.
        let end = self
            .data
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.data.len());

        let s = core::str::from_utf8(&self.data[..end]).ok()?;

        // Advance past the string + null byte.
        if end < self.data.len() {
            self.data = &self.data[end + 1..];
        } else {
            self.data = &[];
        }

        // Skip empty trailing entries.
        if s.is_empty() {
            return None;
        }

        Some(s)
    }
}

/// Iterator over properties within a node.
///
/// Scans `FDT_PROP` tokens starting at a given offset in the structure block,
/// stopping when a non-property token (`FDT_BEGIN_NODE`, `FDT_END_NODE`,
/// `FDT_END`) is reached.
pub struct PropertyIter<'a> {
    struct_block: &'a [u8],
    strings_block: &'a [u8],
    offset: usize,
}

impl<'a> PropertyIter<'a> {
    /// Creates a new property iterator starting at `offset` in the structure block.
    pub(crate) fn new(struct_block: &'a [u8], strings_block: &'a [u8], offset: usize) -> Self {
        Self {
            struct_block,
            strings_block,
            offset,
        }
    }

    /// Returns the current byte offset into the structure block.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }
}

impl<'a> Iterator for PropertyIter<'a> {
    type Item = FdtProperty<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let token = crate::node::read_token_tag(self.struct_block, self.offset)?;

            match token {
                crate::node::FDT_NOP => {
                    self.offset += 4;
                }
                crate::node::FDT_PROP => {
                    // Skip past the token itself.
                    let base = self.offset + 4;
                    let len = crate::node::read_be32_at(self.struct_block, base)? as usize;
                    let nameoff = crate::node::read_be32_at(self.struct_block, base + 4)? as usize;

                    let data_start = base + 8;
                    let data_end = data_start + len;
                    if data_end > self.struct_block.len() {
                        return None;
                    }
                    let data = &self.struct_block[data_start..data_end];

                    // Advance past data + 4-byte alignment padding.
                    self.offset = crate::node::align4(data_end);

                    // Look up the property name in the strings block.
                    let name = crate::node::str_from_offset(self.strings_block, nameoff)?;

                    return Some(FdtProperty::new(name, data));
                }
                // Any non-property token ends the property list.
                _ => return None,
            }
        }
    }
}
