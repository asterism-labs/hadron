//! FDT node types and structure-block token parsing.

use crate::property::{FdtProperty, PropertyIter};

// ---- Token constants --------------------------------------------------------

pub(crate) const FDT_BEGIN_NODE: u32 = 0x0000_0001;
pub(crate) const FDT_END_NODE: u32 = 0x0000_0002;
pub(crate) const FDT_PROP: u32 = 0x0000_0003;
pub(crate) const FDT_NOP: u32 = 0x0000_0004;
#[allow(dead_code)]
pub(crate) const FDT_END: u32 = 0x0000_0009;

// ---- Helpers ----------------------------------------------------------------

/// Reads a big-endian `u32` at `offset` in `data`.
pub(crate) fn read_be32_at(data: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes))
}

/// Reads the 4-byte token tag at `offset`.
pub(crate) fn read_token_tag(struct_block: &[u8], offset: usize) -> Option<u32> {
    read_be32_at(struct_block, offset)
}

/// Rounds `offset` up to the next 4-byte boundary.
pub(crate) fn align4(offset: usize) -> usize {
    (offset + 3) & !3
}

/// Extracts a null-terminated UTF-8 string starting at `offset` in `data`.
pub(crate) fn str_from_offset(data: &[u8], offset: usize) -> Option<&str> {
    let bytes = data.get(offset..)?;
    let end = bytes.iter().position(|&b| b == 0)?;
    core::str::from_utf8(&bytes[..end]).ok()
}

/// Skips over a single node subtree starting just after its `FDT_BEGIN_NODE`
/// name. Returns the offset right after the matching `FDT_END_NODE`.
fn skip_node_subtree(struct_block: &[u8], mut offset: usize) -> Option<usize> {
    let mut depth: u32 = 1;
    while depth > 0 {
        let tag = read_token_tag(struct_block, offset)?;
        offset += 4;

        match tag {
            FDT_BEGIN_NODE => {
                // Skip the null-terminated name + alignment.
                let name_end = struct_block.get(offset..)?.iter().position(|&b| b == 0)?;
                offset = align4(offset + name_end + 1);
                depth += 1;
            }
            FDT_END_NODE => {
                depth -= 1;
            }
            FDT_PROP => {
                let len = read_be32_at(struct_block, offset)? as usize;
                // Skip len(4) + nameoff(4) + data(len) + alignment.
                offset = align4(offset + 8 + len);
            }
            FDT_NOP => {}
            _ => return None,
        }
    }
    Some(offset)
}

// ---- FdtNode ----------------------------------------------------------------

/// A node in the flattened device tree.
///
/// Holds a reference back to the overall FDT data for strings lookup and a
/// position (offset) into the structure block where the node's content
/// (properties and children) begins.
pub struct FdtNode<'a> {
    struct_block: &'a [u8],
    strings_block: &'a [u8],
    name: &'a str,
    /// Offset into `struct_block` right after the node name (where props/children start).
    content_offset: usize,
}

impl<'a> FdtNode<'a> {
    /// Creates a new node.
    pub(crate) fn new(
        struct_block: &'a [u8],
        strings_block: &'a [u8],
        name: &'a str,
        content_offset: usize,
    ) -> Self {
        Self {
            struct_block,
            strings_block,
            name,
            content_offset,
        }
    }

    /// Returns the node name (e.g. `"memory@80000000"` or `""` for root).
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.name
    }

    /// Returns an iterator over the node's properties.
    #[must_use]
    pub fn properties(&self) -> PropertyIter<'a> {
        PropertyIter::new(self.struct_block, self.strings_block, self.content_offset)
    }

    /// Looks up a property by name within this node.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<FdtProperty<'a>> {
        self.properties().find(|p| p.name() == name)
    }

    /// Returns an iterator over this node's direct children.
    #[must_use]
    pub fn children(&self) -> ChildIter<'a> {
        // Skip past properties to find where children begin.
        let mut props =
            PropertyIter::new(self.struct_block, self.strings_block, self.content_offset);
        // Exhaust all properties.
        while props.next().is_some() {}
        ChildIter {
            struct_block: self.struct_block,
            strings_block: self.strings_block,
            offset: props.offset(),
        }
    }

    /// Finds a direct child node by name.
    #[must_use]
    pub fn find_child(&self, name: &str) -> Option<FdtNode<'a>> {
        self.children().find(|n| n.name() == name)
    }

    /// Searches this node's subtree (breadth-first through direct children,
    /// then recursively) for a node whose `compatible` property contains
    /// the given string.
    #[must_use]
    pub fn find_compatible(&self, compatible: &str) -> Option<FdtNode<'a>> {
        for child in self.children() {
            if let Some(prop) = child.property("compatible") {
                if prop.as_str_list().any(|s| s == compatible) {
                    return Some(child);
                }
            }
            if let Some(found) = child.find_compatible(compatible) {
                return Some(found);
            }
        }
        None
    }
}

// ---- ChildIter --------------------------------------------------------------

/// Iterator over the direct children of a node.
///
/// Starts scanning from the first `FDT_BEGIN_NODE` after all properties.
/// Yields child nodes, skipping over their entire subtrees to find siblings.
pub struct ChildIter<'a> {
    struct_block: &'a [u8],
    strings_block: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for ChildIter<'a> {
    type Item = FdtNode<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let tag = read_token_tag(self.struct_block, self.offset)?;

            match tag {
                FDT_BEGIN_NODE => {
                    let name_start = self.offset + 4;
                    let name = str_from_offset(self.struct_block, name_start)?;
                    let content_offset = align4(name_start + name.len() + 1);

                    // Advance past the entire child subtree for the next call.
                    self.offset = skip_node_subtree(self.struct_block, content_offset)?;

                    return Some(FdtNode::new(
                        self.struct_block,
                        self.strings_block,
                        name,
                        content_offset,
                    ));
                }
                FDT_NOP => {
                    self.offset += 4;
                }
                // END_NODE or END means no more children.
                _ => return None,
            }
        }
    }
}
