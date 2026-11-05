//! Segment descriptors (SDM Vol. 3A §3.4.5) and the 16-byte system
//! descriptors of 64-bit mode (§3.5).

use crate::segment::PrivilegeLevel;

// Bits of the low 64-bit word, shared by both descriptor formats.
/// Bits 0..16: limit bits 0..16.
const LIMIT_LOW_MASK: u64 = 0xffff;
/// Bits 48..52: limit bits 16..20.
const LIMIT_HIGH_SHIFT: u32 = 48;
/// Bits 16..40: base bits 0..24.
const BASE_LOW_SHIFT: u32 = 16;
/// Bits 56..64: base bits 24..32.
const BASE_MID_SHIFT: u32 = 56;
/// Bits 40..44: segment type.
const TYPE_SHIFT: u32 = 40;
/// Bit 44: set for code and data, clear for system descriptors.
const CODE_OR_DATA: u64 = 1 << 44;
/// Bits 45..47: descriptor privilege level.
const DPL_SHIFT: u32 = 45;
/// Bit 47: present.
const PRESENT: u64 = 1 << 47;
/// Bit 53: 64-bit code segment.
const LONG_MODE: u64 = 1 << 53;
/// Bit 54: 32-bit default operand size. Must be clear when `LONG_MODE` is set.
const DEFAULT_32: u64 = 1 << 54;
/// Bit 55: the limit counts 4 KiB pages instead of bytes.
const GRANULARITY_4K: u64 = 1 << 55;

// Code and data type bits, within the type field.
/// The CPU sets this on the first load. Setting it up front avoids that
/// write, which would fault on a read-only table.
const TYPE_ACCESSED: u64 = 1 << 0;
/// Readable for code, writable for data.
const TYPE_READ_WRITE: u64 = 1 << 1;
/// Code rather than data.
const TYPE_EXECUTABLE: u64 = 1 << 3;

/// The largest limit a descriptor can hold: 20 bits.
const MAX_LIMIT: u32 = 0xf_ffff;
/// Limit and granularity of a flat segment covering 4 GiB. 64-bit mode
/// ignores both for code and data, but these are the values every 64-bit
/// kernel uses, so the descriptors match what tools expect.
const FLAT_LIMIT: u64 = LIMIT_LOW_MASK | 0xf << LIMIT_HIGH_SHIFT | GRANULARITY_4K;

/// An 8-byte code or data segment descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(transparent)]
pub struct SegmentDescriptor(u64);

impl SegmentDescriptor {
    /// The null descriptor. Loading it into CS or SS faults.
    pub const NULL: Self = Self(0);

    /// A present, readable 64-bit code segment at `dpl`.
    #[must_use]
    pub const fn code64(dpl: PrivilegeLevel) -> Self {
        let ty = TYPE_ACCESSED | TYPE_READ_WRITE | TYPE_EXECUTABLE;
        Self(Self::common(ty, dpl) | LONG_MODE)
    }

    /// A present, writable data segment at `dpl`.
    #[must_use]
    pub const fn data(dpl: PrivilegeLevel) -> Self {
        let ty = TYPE_ACCESSED | TYPE_READ_WRITE;
        Self(Self::common(ty, dpl) | DEFAULT_32)
    }

    const fn common(ty: u64, dpl: PrivilegeLevel) -> u64 {
        FLAT_LIMIT | ty << TYPE_SHIFT | CODE_OR_DATA | (dpl as u64) << DPL_SHIFT | PRESENT
    }

    /// The descriptor privilege level.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to two bits
    pub const fn dpl(self) -> PrivilegeLevel {
        PrivilegeLevel::from_bits((self.0 >> DPL_SHIFT) as u8)
    }

    /// The raw 64-bit descriptor.
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }
}

/// The type of a system descriptor in 64-bit mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SystemType {
    /// A local descriptor table.
    Ldt = 0x2,
    /// A 64-bit TSS that no CPU has loaded.
    TssAvailable = 0x9,
    /// A 64-bit TSS that a CPU has loaded. `ltr` sets this type in the
    /// table, which is why the table must be writable.
    TssBusy = 0xb,
}

impl SystemType {
    /// The type of the system descriptor whose low word is `low`, or `None`
    /// for a type 64-bit mode does not use.
    #[must_use]
    pub const fn of(low: u64) -> Option<Self> {
        match (low >> TYPE_SHIFT) & 0xf {
            0x2 => Some(Self::Ldt),
            0x9 => Some(Self::TssAvailable),
            0xb => Some(Self::TssBusy),
            _ => None,
        }
    }
}

/// A 16-byte system descriptor for a TSS or an LDT. It takes two slots of a
/// GDT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemDescriptor {
    low: u64,
    high: u64,
}

impl SystemDescriptor {
    /// A present, available 64-bit TSS at `base`. `limit` is the TSS size in
    /// bytes minus one.
    ///
    /// # Panics
    ///
    /// Panics if `limit` does not fit in 20 bits.
    #[must_use]
    pub const fn tss(base: u64, limit: u32) -> Self {
        Self::new(SystemType::TssAvailable, base, limit)
    }

    /// A present LDT at `base`. `limit` is the table size in bytes minus
    /// one.
    ///
    /// # Panics
    ///
    /// Panics if `limit` does not fit in 20 bits.
    #[must_use]
    pub const fn ldt(base: u64, limit: u32) -> Self {
        Self::new(SystemType::Ldt, base, limit)
    }

    const fn new(ty: SystemType, base: u64, limit: u32) -> Self {
        assert!(limit <= MAX_LIMIT, "a descriptor limit has 20 bits");
        let limit = limit as u64;
        let low = (limit & LIMIT_LOW_MASK)
            | (base & 0xff_ffff) << BASE_LOW_SHIFT
            | (ty as u64) << TYPE_SHIFT
            | PRESENT
            | (limit >> 16) << LIMIT_HIGH_SHIFT
            | (base >> 24 & 0xff) << BASE_MID_SHIFT;
        Self {
            low,
            high: base >> 32,
        }
    }

    /// The type field.
    ///
    /// # Panics
    ///
    /// Never: the constructors only write types [`SystemType`] names.
    #[must_use]
    pub const fn system_type(self) -> SystemType {
        match SystemType::of(self.low) {
            Some(ty) => ty,
            None => panic!("system descriptor with an unknown type"),
        }
    }

    /// The base address.
    #[must_use]
    pub const fn base(self) -> u64 {
        (self.low >> BASE_LOW_SHIFT & 0xff_ffff)
            | (self.low >> BASE_MID_SHIFT) << 24
            | self.high << 32
    }

    /// The limit in bytes.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // the limit has 20 bits
    pub const fn limit(self) -> u32 {
        ((self.low & LIMIT_LOW_MASK) | (self.low >> LIMIT_HIGH_SHIFT & 0xf) << 16) as u32
    }

    /// The two 64-bit words, low first, as they sit in the table.
    #[must_use]
    pub const fn words(self) -> [u64; 2] {
        [self.low, self.high]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values decoded by hand from SDM Vol. 3A Figure 3-8 and match
    // the GDT entries Linux uses.
    #[test]
    fn code_and_data_match_hand_decoded_values() {
        assert_eq!(
            SegmentDescriptor::code64(PrivilegeLevel::Ring0).bits(),
            0x00af_9b00_0000_ffff
        );
        assert_eq!(
            SegmentDescriptor::data(PrivilegeLevel::Ring0).bits(),
            0x00cf_9300_0000_ffff
        );
        assert_eq!(
            SegmentDescriptor::code64(PrivilegeLevel::Ring3).bits(),
            0x00af_fb00_0000_ffff
        );
        assert_eq!(
            SegmentDescriptor::data(PrivilegeLevel::Ring3).bits(),
            0x00cf_f300_0000_ffff
        );
    }

    #[test]
    fn dpl_reads_back() {
        assert_eq!(
            SegmentDescriptor::code64(PrivilegeLevel::Ring3).dpl(),
            PrivilegeLevel::Ring3
        );
        assert_eq!(
            SegmentDescriptor::data(PrivilegeLevel::Ring0).dpl(),
            PrivilegeLevel::Ring0
        );
    }

    #[test]
    fn tss_descriptor_splits_the_base() {
        let base = 0xffff_ffff_8012_3456;
        let tss = SystemDescriptor::tss(base, 103);
        // Limit 103 (0x67), base bits 0..24 = 0x123456, type 9, present,
        // base bits 24..32 = 0x80, upper word = 0xffff_ffff.
        assert_eq!(tss.words(), [0x8000_8912_3456_0067, 0xffff_ffff]);
        assert_eq!(tss.base(), base);
        assert_eq!(tss.limit(), 103);
        assert_eq!(tss.system_type(), SystemType::TssAvailable);
    }

    #[test]
    fn ldt_descriptor_uses_type_2_and_the_high_limit_bits() {
        let ldt = SystemDescriptor::ldt(0x1000, 0x1_2345);
        assert_eq!(ldt.system_type(), SystemType::Ldt);
        assert_eq!(ldt.limit(), 0x1_2345);
        assert_eq!(ldt.base(), 0x1000);
        assert_eq!(
            ldt.words()[0] >> 44 & 1,
            0,
            "system descriptors clear the S bit"
        );
    }

    #[test]
    fn busy_tss_type_is_recognised() {
        let busy = SystemDescriptor::tss(0, 103).words()[0] | 0x2 << 40;
        assert_eq!(SystemType::of(busy), Some(SystemType::TssBusy));
    }

    #[test]
    #[should_panic(expected = "20 bits")]
    fn limit_past_20_bits_panics() {
        let _ = SystemDescriptor::tss(0, 0x10_0000);
    }
}
