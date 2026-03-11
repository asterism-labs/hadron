//! Shared CPU data structures used by multiple instruction wrappers.
//!
//! These are lightweight types needed by the `instructions` module (segmentation,
//! tables). The full GDT/IDT/TSS types remain in `hadron-kernel` since they
//! contain kernel-specific policy.

/// A segment selector value for the GDT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SegmentSelector(u16);

/// Bit positions and masks for x86_64 segment selectors.
mod segment_bits {
    /// Shift to convert a GDT index to a selector value (skip TI and RPL bits).
    pub const SELECTOR_INDEX_SHIFT: u16 = 3;
    /// Mask for the 2-bit requested privilege level field.
    pub const RPL_MASK: u16 = 0b11;
}

impl SegmentSelector {
    /// Creates a new segment selector.
    ///
    /// `index` is the GDT entry index (0-based), `rpl` is the requested
    /// privilege level (0-3).
    #[inline]
    pub const fn new(index: u16, rpl: u16) -> Self {
        Self((index << segment_bits::SELECTOR_INDEX_SHIFT) | (rpl & segment_bits::RPL_MASK))
    }

    /// Creates a segment selector from a raw `u16` value.
    #[inline]
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// Returns the raw u16 value.
    #[inline]
    pub const fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns the GDT index (bits 3..15).
    #[inline]
    pub const fn index(self) -> u16 {
        self.0 >> segment_bits::SELECTOR_INDEX_SHIFT
    }

    /// Returns the requested privilege level (bits 0..1).
    #[inline]
    pub const fn rpl(self) -> u16 {
        self.0 & segment_bits::RPL_MASK
    }
}

/// Pointer to the GDT or IDT, used by `lgdt` / `lidt`.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct DescriptorTablePointer {
    /// Size of the table minus one.
    pub limit: u16,
    /// Linear base address of the table.
    pub base: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem;

    #[test]
    fn segment_selector_bit_packing_round_trip() {
        let sel = SegmentSelector::new(3, 0);
        assert_eq!(sel.index(), 3);
        assert_eq!(sel.rpl(), 0);
        assert_eq!(sel.as_u16(), 3 << 3);

        let sel_rpl = SegmentSelector::new(5, 3);
        assert_eq!(sel_rpl.index(), 5);
        assert_eq!(sel_rpl.rpl(), 3);
        assert_eq!(sel_rpl.as_u16(), (5 << 3) | 3);
    }

    #[test]
    fn segment_selector_from_raw_round_trip() {
        let raw = 0x28_u16; // GDT index 5, RPL 0
        let sel = SegmentSelector::from_raw(raw);
        assert_eq!(sel.as_u16(), raw);
        assert_eq!(sel.index(), 5);
        assert_eq!(sel.rpl(), 0);
    }

    #[test]
    fn segment_selector_rpl_mask() {
        // RPL should be masked to 2 bits
        let sel = SegmentSelector::new(0, 0xFF);
        assert_eq!(sel.rpl(), 3);
    }

    #[test]
    fn descriptor_table_pointer_size() {
        assert_eq!(mem::size_of::<DescriptorTablePointer>(), 10);
    }
}
