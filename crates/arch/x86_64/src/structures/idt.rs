//! The 64-bit IDT and its gate descriptors (SDM Vol. 3A §6.14.1).

use core::mem::size_of;

use crate::structures::segment::{PrivilegeLevel, SegmentSelector};
use crate::structures::table::DescriptorTablePointer;

/// Number of vectors in an IDT.
pub const VECTORS: usize = 256;

/// Attribute bit 7: present.
const PRESENT: u8 = 1 << 7;
/// Attribute bits 5 and 6: descriptor privilege level.
const DPL_SHIFT: u8 = 5;
/// The IST field uses bits 0 to 2 of its byte.
const IST_MASK: u8 = 0b111;

/// The kind of an IDT gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GateType {
    /// Clears IF on entry, so the handler runs with interrupts masked.
    Interrupt = 0xe,
    /// Leaves IF as it was.
    Trap = 0xf,
}

/// An interrupt stack table slot, IST1 to IST7. A gate that names one
/// always switches to that stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IstIndex(u8);

impl IstIndex {
    /// IST slot `n`, from 1 to 7.
    ///
    /// # Panics
    ///
    /// Panics if `n` is 0 or above 7.
    #[must_use]
    pub const fn new(n: u8) -> Self {
        assert!(n >= 1 && n <= 7, "IST slots are numbered 1 to 7");
        Self(n)
    }

    /// The slot number, from 1 to 7.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// A 16-byte IDT gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct GateDescriptor {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl GateDescriptor {
    /// A gate that is not present. Its vector raises #GP when delivered.
    pub const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_mid: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// A present gate that enters `handler` through code segment
    /// `selector`. `dpl` is the most privileged ring allowed to raise the
    /// vector with `int`. With `ist`, the CPU always switches to that
    /// stack.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // each field takes its own bits
    pub const fn new(
        handler: u64,
        selector: SegmentSelector,
        gate_type: GateType,
        dpl: PrivilegeLevel,
        ist: Option<IstIndex>,
    ) -> Self {
        Self {
            offset_low: handler as u16,
            selector: selector.bits(),
            ist: match ist {
                Some(index) => index.get(),
                None => 0,
            },
            attributes: PRESENT | (dpl as u8) << DPL_SHIFT | gate_type as u8,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }

    /// The handler address.
    #[must_use]
    pub const fn handler(self) -> u64 {
        self.offset_low as u64 | (self.offset_mid as u64) << 16 | (self.offset_high as u64) << 32
    }

    /// The code segment the CPU loads.
    #[must_use]
    pub const fn selector(self) -> SegmentSelector {
        SegmentSelector::from_bits(self.selector)
    }

    /// Whether the gate is present.
    #[must_use]
    pub const fn is_present(self) -> bool {
        self.attributes & PRESENT != 0
    }

    /// The IST slot, if the gate names one.
    #[must_use]
    pub const fn ist(self) -> Option<IstIndex> {
        match self.ist & IST_MASK {
            0 => None,
            n => Some(IstIndex(n)),
        }
    }

    /// The gate type, or `None` for a gate that is not present.
    #[must_use]
    pub const fn gate_type(self) -> Option<GateType> {
        match (self.is_present(), self.attributes & 0xf) {
            (true, 0xe) => Some(GateType::Interrupt),
            (true, 0xf) => Some(GateType::Trap),
            _ => None,
        }
    }

    /// The descriptor privilege level.
    #[must_use]
    pub const fn dpl(self) -> PrivilegeLevel {
        PrivilegeLevel::from_bits(self.attributes >> DPL_SHIFT)
    }
}

/// An IDT with a gate for each of the 256 vectors.
#[derive(Debug)]
#[repr(C, align(16))]
pub struct Idt {
    gates: [GateDescriptor; VECTORS],
}

impl Idt {
    /// An IDT with every gate missing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            gates: [GateDescriptor::MISSING; VECTORS],
        }
    }

    /// Sets the gate for `vector`.
    pub const fn set(&mut self, vector: u8, gate: GateDescriptor) {
        self.gates[vector as usize] = gate;
    }

    /// The gate for `vector`.
    #[must_use]
    pub const fn gate(&self, vector: u8) -> GateDescriptor {
        self.gates[vector as usize]
    }

    /// The operand for `lidt`. The IDT must not move or be dropped while a
    /// CPU uses it.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // 4095
    pub fn pointer(&self) -> DescriptorTablePointer {
        DescriptorTablePointer {
            limit: (size_of::<Self>() - 1) as u16,
            base: self.gates.as_ptr() as u64,
        }
    }
}

impl Default for Idt {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structures::segment::TableIndicator;

    const KERNEL_CODE: SegmentSelector =
        SegmentSelector::new(1, TableIndicator::Global, PrivilegeLevel::Ring0);

    #[test]
    fn gate_is_16_bytes_and_idt_is_4_kib() {
        assert_eq!(size_of::<GateDescriptor>(), 16);
        assert_eq!(size_of::<Idt>(), 4096);
        assert_eq!(align_of::<Idt>(), 16);
    }

    #[test]
    fn interrupt_gate_matches_hand_decoded_bytes() {
        let gate = GateDescriptor::new(
            0xffff_ffff_8012_3456,
            KERNEL_CODE,
            GateType::Interrupt,
            PrivilegeLevel::Ring0,
            Some(IstIndex::new(1)),
        );
        // Offset 0x3456, selector 0x08, IST 1, attributes 0x8e (present,
        // DPL 0, interrupt gate), offset 0x8012, offset 0xffff_ffff.
        let bytes: [u8; 16] = unsafe { core::mem::transmute(gate) };
        assert_eq!(
            bytes,
            [
                0x56, 0x34, 0x08, 0x00, 0x01, 0x8e, 0x12, 0x80, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn fields_read_back() {
        let gate = GateDescriptor::new(
            0xffff_8000_dead_beef,
            KERNEL_CODE,
            GateType::Trap,
            PrivilegeLevel::Ring3,
            None,
        );
        assert_eq!(gate.handler(), 0xffff_8000_dead_beef);
        assert_eq!(gate.selector(), KERNEL_CODE);
        assert!(gate.is_present());
        assert_eq!(gate.ist(), None);
        assert_eq!(gate.gate_type(), Some(GateType::Trap));
        assert_eq!(gate.dpl(), PrivilegeLevel::Ring3);
    }

    #[test]
    fn missing_gate_is_not_present() {
        assert!(!GateDescriptor::MISSING.is_present());
        assert_eq!(GateDescriptor::MISSING.gate_type(), None);
    }

    #[test]
    fn pointer_covers_all_vectors() {
        let idt = Idt::new();
        assert_eq!({ idt.pointer().limit }, 4095);
    }

    #[test]
    #[should_panic(expected = "1 to 7")]
    fn ist_zero_panics() {
        let _ = IstIndex::new(0);
    }
}
