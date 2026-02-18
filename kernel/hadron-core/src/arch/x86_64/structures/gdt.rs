//! Global Descriptor Table (GDT) structures.

use core::mem::size_of;

/// Bit positions and masks for x86_64 segment descriptors.
mod segment_bits {
    /// Shift to convert a GDT index to a selector value (skip TI and RPL bits).
    pub const SELECTOR_INDEX_SHIFT: u16 = 3;
    /// Mask for the 2-bit requested privilege level field.
    pub const RPL_MASK: u16 = 0b11;
    /// Bit position of the DPL field in a segment descriptor.
    pub const DPL_SHIFT: u64 = 45;
    /// Mask for the 2-bit DPL field (after shifting).
    pub const DPL_MASK: u64 = 0b11;
}

/// A segment selector value for the GDT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct SegmentSelector(u16);

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

/// A GDT descriptor entry.
#[derive(Debug, Clone, Copy)]
pub enum Descriptor {
    /// A 64-bit user segment (code/data) or null descriptor.
    UserSegment(u64),
    /// A 128-bit system segment (e.g., TSS) — stores low and high halves.
    SystemSegment(u64, u64),
}

impl Descriptor {
    /// Creates a null descriptor.
    #[inline]
    pub const fn null() -> Self {
        Self::UserSegment(0)
    }

    /// Creates a 64-bit kernel code segment descriptor.
    ///
    /// L=1, D=0, P=1, DPL=0, type=execute/read.
    #[inline]
    pub const fn kernel_code_segment() -> Self {
        Self::UserSegment(0x00AF_9A00_0000_FFFF)
    }

    /// Creates a kernel data segment descriptor.
    ///
    /// P=1, DPL=0, type=read/write.
    #[inline]
    pub const fn kernel_data_segment() -> Self {
        Self::UserSegment(0x00CF_9200_0000_FFFF)
    }

    /// Creates a 64-bit user code segment descriptor.
    ///
    /// L=1, D=0, P=1, DPL=3, type=execute/read.
    #[inline]
    pub const fn user_code_segment() -> Self {
        Self::UserSegment(0x00AF_FA00_0000_FFFF)
    }

    /// Creates a user data segment descriptor.
    ///
    /// P=1, DPL=3, type=read/write.
    #[inline]
    pub const fn user_data_segment() -> Self {
        Self::UserSegment(0x00CF_F200_0000_FFFF)
    }

    /// TSS type: 64-bit TSS (available).
    const TSS_TYPE_AVAILABLE_64: u64 = 0x9;
    /// Bit position of the Present flag in a segment descriptor.
    const TSS_PRESENT_BIT: u64 = 47;

    /// Creates a 128-bit TSS system segment descriptor from a static TSS reference.
    pub fn tss_segment(tss: &'static TaskStateSegment) -> Self {
        let tss_ptr = tss as *const _ as u64;
        let limit = (size_of::<TaskStateSegment>() - 1) as u64;

        // Low 64 bits of TSS descriptor:
        //  bits  0..15: limit[0..15]
        //  bits 16..39: base[0..23]
        //  bits 40..43: type (0x9 = 64-bit TSS available)
        //  bit      44: 0 (system segment)
        //  bits 45..46: DPL (0)
        //  bit      47: present (1)
        //  bits 48..51: limit[16..19]
        //  bit      52: AVL (0)
        //  bits 53..54: reserved (0)
        //  bit      55: granularity (0)
        //  bits 56..63: base[24..31]
        let low = (limit & 0xFFFF)
            | ((tss_ptr & 0xFFFFFF) << 16)
            | (Self::TSS_TYPE_AVAILABLE_64 << 40)
            | (1 << Self::TSS_PRESENT_BIT)
            | ((limit & 0xF0000) << 32)
            | ((tss_ptr & 0xFF000000) << 32);

        // High 64 bits: base[32..63] and reserved
        let high = (tss_ptr >> 32) & 0xFFFF_FFFF;

        Self::SystemSegment(low, high)
    }

    /// Returns the privilege level (DPL) of this descriptor (bits 45-46 of the low word).
    fn privilege_level(&self) -> u16 {
        let low = match self {
            Self::UserSegment(bits) => *bits,
            Self::SystemSegment(bits, _) => *bits,
        };
        ((low >> segment_bits::DPL_SHIFT) & segment_bits::DPL_MASK) as u16
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

/// Global Descriptor Table with a fixed maximum capacity of `N` 64-bit slots.
///
/// The default capacity of 8 supports: null + kernel_code + kernel_data +
/// user_code + user_data + TSS (2 slots) = 7 entries, with 1 spare.
#[repr(C, align(16))]
pub struct GlobalDescriptorTable<const N: usize = 8> {
    table: [u64; N],
    len: usize,
}

impl<const N: usize> GlobalDescriptorTable<N> {
    /// Creates a new GDT with only a null descriptor in slot 0.
    pub const fn new() -> Self {
        let mut table = [0u64; N];
        table[0] = 0; // null descriptor
        Self { table, len: 1 }
    }

    /// Appends a descriptor to the GDT and returns the corresponding
    /// [`SegmentSelector`].
    ///
    /// # Panics
    ///
    /// Panics if the table is full.
    pub fn append(&mut self, descriptor: Descriptor) -> SegmentSelector {
        let index = self.len;
        let rpl = descriptor.privilege_level();

        match descriptor {
            Descriptor::UserSegment(bits) => {
                assert!(index < N, "GDT full");
                self.table[index] = bits;
                self.len += 1;
            }
            Descriptor::SystemSegment(low, high) => {
                assert!(index + 1 < N, "GDT full (need 2 slots for system segment)");
                self.table[index] = low;
                self.table[index + 1] = high;
                self.len += 2;
            }
        }

        SegmentSelector::new(index as u16, rpl)
    }

    /// Loads this GDT into the CPU via the `lgdt` instruction.
    ///
    /// # Safety
    ///
    /// - The GDT must be `'static` (must not be dropped while loaded).
    /// - The caller must ensure the descriptors are valid.
    /// - Segment registers must be reloaded after this call.
    #[inline]
    pub unsafe fn load(&'static self) {
        let ptr = DescriptorTablePointer {
            limit: (self.len * size_of::<u64>() - 1) as u16,
            base: self.table.as_ptr() as u64,
        };
        unsafe {
            core::arch::asm!(
                "lgdt [{}]",
                in(reg) &ptr,
                options(readonly, nostack, preserves_flags),
            );
        }
    }
}

/// Task State Segment (TSS) for x86_64.
///
/// Contains the interrupt stack table (IST) and privilege stack table used by
/// the CPU during stack switches.
#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct TaskStateSegment {
    _reserved_0: u32,
    /// Privilege stack table (RSP values for ring 0-2).
    pub privilege_stack_table: [u64; 3],
    _reserved_1: u64,
    /// Interrupt stack table (IST1-IST7).
    pub interrupt_stack_table: [u64; 7],
    _reserved_2: u64,
    _reserved_3: u16,
    /// Offset from the TSS base to the I/O permission bitmap.
    pub iomap_base: u16,
}

impl TaskStateSegment {
    /// Creates a new zeroed TSS.
    pub const fn new() -> Self {
        Self {
            _reserved_0: 0,
            privilege_stack_table: [0; 3],
            _reserved_1: 0,
            interrupt_stack_table: [0; 7],
            _reserved_2: 0,
            _reserved_3: 0,
            iomap_base: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_encoding() {
        let sel = SegmentSelector::new(1, 0);
        assert_eq!(sel.as_u16(), 0x08);
    }

    #[test]
    fn selector_with_rpl() {
        let sel = SegmentSelector::new(2, 3);
        assert_eq!(sel.as_u16(), (2 << 3) | 3);
        assert_eq!(sel.index(), 2);
        assert_eq!(sel.rpl(), 3);
    }

    #[test]
    fn selector_from_raw() {
        let sel = SegmentSelector::from_raw(0x18);
        assert_eq!(sel.index(), 3);
        assert_eq!(sel.rpl(), 0);
    }

    #[test]
    fn selector_rpl_masked() {
        // RPL is only 2 bits; high bits should be masked off.
        let sel = SegmentSelector::new(1, 0xFF);
        assert_eq!(sel.rpl(), 3); // 0xFF & 0b11 = 3
    }

    #[test]
    fn gdt_append_kernel_code() {
        let mut gdt = GlobalDescriptorTable::<8>::new();
        let sel = gdt.append(Descriptor::kernel_code_segment());
        assert_eq!(sel.index(), 1);
        assert_eq!(sel.rpl(), 0);
    }

    #[test]
    fn gdt_append_sequential() {
        let mut gdt = GlobalDescriptorTable::<8>::new();
        let kc = gdt.append(Descriptor::kernel_code_segment());
        let kd = gdt.append(Descriptor::kernel_data_segment());
        let uc = gdt.append(Descriptor::user_code_segment());
        let ud = gdt.append(Descriptor::user_data_segment());
        assert_eq!(kc.index(), 1);
        assert_eq!(kd.index(), 2);
        assert_eq!(uc.index(), 3);
        assert_eq!(ud.index(), 4);
    }

    #[test]
    fn kernel_code_segment_bits() {
        let desc = Descriptor::kernel_code_segment();
        let bits = match desc {
            Descriptor::UserSegment(b) => b,
            _ => panic!("expected UserSegment"),
        };
        // P=1 (bit 47)
        assert_ne!(bits & (1 << 47), 0, "present bit not set");
        // L=1 (bit 53) — 64-bit code segment
        assert_ne!(bits & (1 << 53), 0, "long mode bit not set");
        // D=0 (bit 54) — must be 0 for 64-bit
        assert_eq!(bits & (1 << 54), 0, "D bit should be 0 for 64-bit");
        // DPL=0 (bits 45-46)
        assert_eq!((bits >> 45) & 0b11, 0, "DPL should be 0");
    }

    #[test]
    fn user_code_dpl_3() {
        let desc = Descriptor::user_code_segment();
        let bits = match desc {
            Descriptor::UserSegment(b) => b,
            _ => panic!("expected UserSegment"),
        };
        assert_eq!((bits >> 45) & 0b11, 3, "DPL should be 3 for user code");
    }

    #[test]
    #[should_panic(expected = "GDT full")]
    fn gdt_overflow_panics() {
        let mut gdt = GlobalDescriptorTable::<2>::new();
        // Slot 0 is null (len=1), slot 1 is the first append.
        gdt.append(Descriptor::kernel_code_segment()); // fills slot 1
        gdt.append(Descriptor::kernel_data_segment()); // should panic
    }

    #[test]
    fn tss_zeroed() {
        let tss = TaskStateSegment::new();
        // Copy fields to local variables to avoid misaligned references on packed struct.
        let pst = { tss.privilege_stack_table };
        let ist = { tss.interrupt_stack_table };
        let iomap = { tss.iomap_base };
        assert_eq!(pst, [0; 3]);
        assert_eq!(ist, [0; 7]);
        assert_eq!(iomap, 0);
    }

    #[test]
    fn tss_size_104_bytes() {
        assert_eq!(
            size_of::<TaskStateSegment>(),
            104,
            "TSS must be exactly 104 bytes per x86_64 spec"
        );
    }

    #[test]
    fn null_descriptor_is_zero() {
        let desc = Descriptor::null();
        let bits = match desc {
            Descriptor::UserSegment(b) => b,
            _ => panic!("expected UserSegment"),
        };
        assert_eq!(bits, 0);
    }
}
