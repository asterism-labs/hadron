//! The GDT and LDT, and the pointer `lgdt` and `lidt` take.

use core::marker::PhantomData;
use core::mem::size_of;

use crate::structures::descriptor::{SegmentDescriptor, SystemDescriptor};
use crate::structures::segment::{PrivilegeLevel, SegmentSelector, TableIndicator};

/// The most slots a descriptor table can hold.
const MAX_SLOTS: usize = 8192;

/// The operand of `lgdt`, `lidt`, `sgdt` and `sidt`: the table's linear
/// address and its size in bytes minus one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, packed)]
pub struct DescriptorTablePointer {
    /// Size of the table in bytes, minus one.
    pub limit: u16,
    /// Linear address of the first slot.
    pub base: u64,
}

/// Which table a [`DescriptorTable`] is. Sealed: [`Global`] and [`Local`]
/// are the only kinds.
pub trait TableKind: sealed::Sealed {
    /// The table indicator in the selectors this table hands out.
    const INDICATOR: TableIndicator;
}

/// Marks a [`DescriptorTable`] as the GDT.
#[derive(Debug)]
pub enum Global {}

/// Marks a [`DescriptorTable`] as an LDT.
#[derive(Debug)]
pub enum Local {}

impl TableKind for Global {
    const INDICATOR: TableIndicator = TableIndicator::Global;
}

impl TableKind for Local {
    const INDICATOR: TableIndicator = TableIndicator::Local;
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Global {}
    impl Sealed for super::Local {}
}

/// A global descriptor table of `N` 8-byte slots.
pub type Gdt<const N: usize> = DescriptorTable<Global, N>;

/// A local descriptor table of `N` 8-byte slots.
pub type Ldt<const N: usize> = DescriptorTable<Local, N>;

/// A descriptor table of `N` 8-byte slots. Callers place each descriptor at
/// a fixed index, and get back the selector for it.
///
/// A GDT keeps slot 0 null, and a system descriptor takes two slots. An LDT
/// holds only code and data descriptors, since 64-bit mode does not allow
/// TSS or LDT descriptors in an LDT.
#[derive(Debug)]
#[repr(C, align(16))]
pub struct DescriptorTable<K: TableKind, const N: usize> {
    slots: [u64; N],
    kind: PhantomData<K>,
}

impl<K: TableKind, const N: usize> DescriptorTable<K, N> {
    /// A table with every slot zero, which is a descriptor that is not
    /// present.
    #[must_use]
    pub const fn new() -> Self {
        const {
            assert!(
                N > 0 && N <= MAX_SLOTS,
                "a descriptor table holds 1 to 8192 slots"
            );
        }
        Self {
            slots: [0; N],
            kind: PhantomData,
        }
    }

    /// Puts `descriptor` at `index` and returns its selector, whose RPL is
    /// the descriptor's DPL.
    ///
    /// # Panics
    ///
    /// Panics if `index` is past the table, or is slot 0 of a GDT.
    pub const fn set_segment(
        &mut self,
        index: u16,
        descriptor: SegmentDescriptor,
    ) -> SegmentSelector {
        Self::check_slots(index, 1);
        self.slots[index as usize] = descriptor.bits();
        SegmentSelector::new(index, K::INDICATOR, descriptor.dpl())
    }

    /// The raw 64-bit word in slot `index`. The second slot of a system
    /// descriptor holds the upper half of its base.
    ///
    /// # Panics
    ///
    /// Panics if `index` is past the table.
    #[must_use]
    pub const fn slot(&self, index: u16) -> u64 {
        self.slots[index as usize]
    }

    /// The operand for loading this table. The table must not move or be
    /// dropped while a CPU uses it.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // N <= 8192, so the size fits
    pub fn pointer(&self) -> DescriptorTablePointer {
        DescriptorTablePointer {
            limit: (N * size_of::<u64>() - 1) as u16,
            base: self.slots.as_ptr() as u64,
        }
    }

    const fn check_slots(index: u16, count: usize) {
        assert!(
            index as usize + count <= N,
            "descriptor does not fit in the table"
        );
        if matches!(K::INDICATOR, TableIndicator::Global) {
            assert!(index != 0, "GDT slot 0 must stay null");
        }
    }
}

impl<const N: usize> DescriptorTable<Global, N> {
    /// Puts the 16-byte `descriptor` in slots `index` and `index + 1` and
    /// returns its selector at ring 0.
    ///
    /// # Panics
    ///
    /// Panics if either slot is past the table, or `index` is slot 0.
    pub const fn set_system(
        &mut self,
        index: u16,
        descriptor: SystemDescriptor,
    ) -> SegmentSelector {
        Self::check_slots(index, 2);
        let [low, high] = descriptor.words();
        self.slots[index as usize] = low;
        self.slots[index as usize + 1] = high;
        SegmentSelector::new(index, TableIndicator::Global, PrivilegeLevel::Ring0)
    }
}

impl<K: TableKind, const N: usize> Default for DescriptorTable<K, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structures::descriptor::SystemType;

    #[test]
    fn pointer_is_ten_bytes() {
        assert_eq!(size_of::<DescriptorTablePointer>(), 10);
    }

    #[test]
    fn gdt_hands_out_selectors_with_the_descriptor_dpl() {
        let mut gdt = Gdt::<8>::new();
        let code = gdt.set_segment(1, SegmentDescriptor::code64(PrivilegeLevel::Ring0));
        let user = gdt.set_segment(4, SegmentDescriptor::data(PrivilegeLevel::Ring3));
        assert_eq!(code.bits(), 0x08);
        assert_eq!(user.bits(), 0x23);
        assert_eq!(
            gdt.slot(1),
            SegmentDescriptor::code64(PrivilegeLevel::Ring0).bits()
        );
        assert_eq!(gdt.slot(0), 0);
    }

    #[test]
    fn system_descriptor_takes_two_slots() {
        let mut gdt = Gdt::<8>::new();
        let tss = SystemDescriptor::tss(0xffff_ffff_8000_1000, 103);
        let selector = gdt.set_system(6, tss);
        assert_eq!(selector.bits(), 0x30);
        assert_eq!(SystemType::of(gdt.slot(6)), Some(SystemType::TssAvailable));
        assert_eq!(gdt.slot(7), 0xffff_ffff);
    }

    #[test]
    fn ldt_selectors_set_the_table_indicator() {
        let mut ldt = Ldt::<4>::new();
        let selector = ldt.set_segment(0, SegmentDescriptor::data(PrivilegeLevel::Ring3));
        assert_eq!(selector.table(), TableIndicator::Local);
        assert_eq!(selector.bits(), 0b111);
    }

    #[test]
    fn pointer_covers_every_slot() {
        let gdt = Gdt::<8>::new();
        let pointer = gdt.pointer();
        assert_eq!({ pointer.limit }, 63);
        assert_eq!({ pointer.base }, core::ptr::from_ref(&gdt) as u64);
    }

    #[test]
    fn table_is_16_byte_aligned() {
        assert_eq!(align_of::<Gdt<8>>(), 16);
        assert_eq!(size_of::<Gdt<8>>(), 64);
    }

    #[test]
    fn tables_build_at_compile_time() {
        const GDT: Gdt<3> = {
            let mut gdt = Gdt::new();
            gdt.set_segment(1, SegmentDescriptor::code64(PrivilegeLevel::Ring0));
            gdt.set_segment(2, SegmentDescriptor::data(PrivilegeLevel::Ring0));
            gdt
        };
        assert_eq!(
            GDT.slot(2),
            SegmentDescriptor::data(PrivilegeLevel::Ring0).bits()
        );
    }

    #[test]
    #[should_panic(expected = "slot 0 must stay null")]
    fn gdt_slot_zero_is_reserved() {
        Gdt::<8>::new().set_segment(0, SegmentDescriptor::code64(PrivilegeLevel::Ring0));
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn system_descriptor_in_the_last_slot_panics() {
        Gdt::<8>::new().set_system(7, SystemDescriptor::tss(0, 103));
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn index_past_the_table_panics() {
        Ldt::<4>::new().set_segment(4, SegmentDescriptor::data(PrivilegeLevel::Ring3));
    }
}
