//! Privilege levels and segment selectors (SDM Vol. 3A §3.4.2).

/// A CPU privilege level, or ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PrivilegeLevel {
    /// The kernel's ring.
    Ring0 = 0,
    /// Unused by Hadron.
    Ring1 = 1,
    /// Unused by Hadron.
    Ring2 = 2,
    /// The user ring.
    Ring3 = 3,
}

impl PrivilegeLevel {
    /// The level in the two low bits of `bits`.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0 => Self::Ring0,
            1 => Self::Ring1,
            2 => Self::Ring2,
            _ => Self::Ring3,
        }
    }
}

/// Which table a selector indexes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TableIndicator {
    /// The global descriptor table.
    Global = 0,
    /// The local descriptor table.
    Local = 1,
}

/// Bit 2 of a selector: set for the LDT.
const TI_BIT: u16 = 1 << 2;
/// The index starts at bit 3.
const INDEX_SHIFT: u16 = 3;
/// The largest index: a table holds at most 8192 slots.
const MAX_INDEX: u16 = (1 << 13) - 1;

/// A segment selector: a table index, a table indicator and a requested
/// privilege level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct SegmentSelector(u16);

impl SegmentSelector {
    /// The null selector: GDT index 0 at ring 0.
    pub const NULL: Self = Self(0);

    /// Builds a selector.
    ///
    /// # Panics
    ///
    /// Panics if `index` is above 8191.
    #[must_use]
    pub const fn new(index: u16, table: TableIndicator, rpl: PrivilegeLevel) -> Self {
        assert!(
            index <= MAX_INDEX,
            "a descriptor table holds at most 8192 slots"
        );
        let ti = match table {
            TableIndicator::Global => 0,
            TableIndicator::Local => TI_BIT,
        };
        Self(index << INDEX_SHIFT | ti | rpl as u16)
    }

    /// Wraps the raw bits of a selector, as read from a segment register.
    #[must_use]
    pub const fn from_bits(bits: u16) -> Self {
        Self(bits)
    }

    /// The raw bits, as loaded into a segment register.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// The slot index in the table.
    #[must_use]
    pub const fn index(self) -> u16 {
        self.0 >> INDEX_SHIFT
    }

    /// The table the selector indexes.
    #[must_use]
    pub const fn table(self) -> TableIndicator {
        if self.0 & TI_BIT == 0 {
            TableIndicator::Global
        } else {
            TableIndicator::Local
        }
    }

    /// The requested privilege level.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to two bits
    pub const fn rpl(self) -> PrivilegeLevel {
        PrivilegeLevel::from_bits(self.0 as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_and_user_selectors_have_the_usual_values() {
        let kernel_code = SegmentSelector::new(1, TableIndicator::Global, PrivilegeLevel::Ring0);
        let user_data = SegmentSelector::new(4, TableIndicator::Global, PrivilegeLevel::Ring3);
        assert_eq!(kernel_code.bits(), 0x08);
        assert_eq!(user_data.bits(), 0x23);
    }

    #[test]
    fn fields_round_trip() {
        let selector = SegmentSelector::new(5, TableIndicator::Local, PrivilegeLevel::Ring3);
        assert_eq!(selector.bits(), 5 << 3 | 1 << 2 | 3);
        assert_eq!(selector.index(), 5);
        assert_eq!(selector.table(), TableIndicator::Local);
        assert_eq!(selector.rpl(), PrivilegeLevel::Ring3);
        assert_eq!(SegmentSelector::from_bits(selector.bits()), selector);
    }

    #[test]
    fn largest_index_fits() {
        let selector = SegmentSelector::new(8191, TableIndicator::Global, PrivilegeLevel::Ring0);
        assert_eq!(selector.index(), 8191);
    }

    #[test]
    #[should_panic(expected = "at most 8192 slots")]
    fn index_past_the_table_panics() {
        let _ = SegmentSelector::new(8192, TableIndicator::Global, PrivilegeLevel::Ring0);
    }
}
