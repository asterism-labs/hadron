//! The 64-bit task state segment.
//!
//! In 64-bit mode the CPU reads the TSS only for stack switches: RSP0 to
//! RSP2 when an interrupt raises the privilege level, and IST1 to IST7 for
//! IDT entries that name an interrupt stack.

use core::mem::size_of;

use crate::structures::idt::IstIndex;

/// The 104-byte 64-bit TSS. Its 8-byte fields sit at 4-byte offsets, so
/// the struct is packed to 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C, packed(4))]
pub struct TaskStateSegment {
    reserved_0: u32,
    /// Stack pointers loaded on a switch to ring 0, 1 and 2.
    pub privilege_stacks: [u64; 3],
    reserved_1: u64,
    /// Interrupt stacks IST1 to IST7. Index 0 is IST1.
    pub interrupt_stacks: [u64; 7],
    reserved_2: u64,
    reserved_3: u16,
    /// Offset of the I/O permission bitmap from the TSS base.
    pub iomap_base: u16,
}

impl TaskStateSegment {
    /// Size of the TSS in bytes.
    #[allow(clippy::cast_possible_truncation)] // 104
    pub const SIZE: u16 = size_of::<Self>() as u16;

    /// A TSS with no stacks and no I/O permission bitmap. An I/O map base at
    /// or past the limit means there is no bitmap, so ring 3 cannot use I/O
    /// ports.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reserved_0: 0,
            privilege_stacks: [0; 3],
            reserved_1: 0,
            interrupt_stacks: [0; 7],
            reserved_2: 0,
            reserved_3: 0,
            iomap_base: Self::SIZE,
        }
    }

    /// Sets the stack the CPU switches to for gates that name `index`.
    /// `top` is the address just past the stack's highest byte.
    pub const fn set_interrupt_stack(&mut self, index: IstIndex, top: u64) {
        self.interrupt_stacks[index.get() as usize - 1] = top;
    }

    /// The limit for this TSS's descriptor: its size minus one.
    #[must_use]
    pub const fn limit() -> u32 {
        Self::SIZE as u32 - 1
    }
}

impl Default for TaskStateSegment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    #[test]
    fn layout_matches_the_sdm() {
        assert_eq!(size_of::<TaskStateSegment>(), 104);
        assert_eq!(offset_of!(TaskStateSegment, privilege_stacks), 4);
        assert_eq!(offset_of!(TaskStateSegment, interrupt_stacks), 36);
        assert_eq!(offset_of!(TaskStateSegment, iomap_base), 102);
    }

    #[test]
    fn ist_1_is_the_first_interrupt_stack() {
        let mut tss = TaskStateSegment::new();
        tss.set_interrupt_stack(IstIndex::new(1), 0x1000);
        tss.set_interrupt_stack(IstIndex::new(7), 0x7000);
        let stacks = tss.interrupt_stacks;
        assert_eq!(stacks[0], 0x1000);
        assert_eq!(stacks[6], 0x7000);
    }

    #[test]
    fn new_tss_has_no_io_bitmap() {
        assert_eq!({ TaskStateSegment::new().iomap_base }, 104);
        assert_eq!(TaskStateSegment::limit(), 103);
    }
}
