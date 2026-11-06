//! The CPU exceptions, vectors 0 to 31 (SDM Vol. 3A §6.3, Table 6-1).

/// A CPU exception vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Exception {
    /// #DE: division by zero or quotient overflow.
    DivideError = 0,
    /// #DB: debug trap or fault.
    Debug = 1,
    /// Non-maskable interrupt.
    Nmi = 2,
    /// #BP: the `int3` instruction.
    Breakpoint = 3,
    /// #OF: the `into` instruction, which 64-bit mode does not have.
    Overflow = 4,
    /// #BR: the `bound` instruction, which 64-bit mode does not have.
    BoundRange = 5,
    /// #UD: undefined or reserved opcode.
    InvalidOpcode = 6,
    /// #NM: FPU or SSE use while CR0.TS or CR0.EM is set.
    DeviceNotAvailable = 7,
    /// #DF: an exception while delivering another one.
    DoubleFault = 8,
    /// #TS: bad TSS during a task switch or stack switch.
    InvalidTss = 10,
    /// #NP: a segment or gate that is not present.
    SegmentNotPresent = 11,
    /// #SS: stack segment fault.
    StackSegmentFault = 12,
    /// #GP: general protection fault.
    GeneralProtection = 13,
    /// #PF: page fault. CR2 holds the faulting address.
    PageFault = 14,
    /// #MF: x87 floating-point error.
    X87FloatingPoint = 16,
    /// #AC: unaligned access with alignment checking on.
    AlignmentCheck = 17,
    /// #MC: machine check.
    MachineCheck = 18,
    /// #XM: SIMD floating-point exception.
    SimdFloatingPoint = 19,
    /// #VE: EPT violation in a guest.
    Virtualization = 20,
    /// #CP: control-flow protection violation.
    ControlProtection = 21,
    /// #HV: hypervisor injection (AMD SEV-SNP).
    HypervisorInjection = 28,
    /// #VC: VMM communication (AMD SEV-ES).
    VmmCommunication = 29,
    /// #SX: security exception (AMD).
    Security = 30,
}

impl Exception {
    /// The exception for `vector`, or `None` for a reserved vector or one
    /// past 31.
    #[must_use]
    pub const fn from_vector(vector: u64) -> Option<Self> {
        Some(match vector {
            0 => Self::DivideError,
            1 => Self::Debug,
            2 => Self::Nmi,
            3 => Self::Breakpoint,
            4 => Self::Overflow,
            5 => Self::BoundRange,
            6 => Self::InvalidOpcode,
            7 => Self::DeviceNotAvailable,
            8 => Self::DoubleFault,
            10 => Self::InvalidTss,
            11 => Self::SegmentNotPresent,
            12 => Self::StackSegmentFault,
            13 => Self::GeneralProtection,
            14 => Self::PageFault,
            16 => Self::X87FloatingPoint,
            17 => Self::AlignmentCheck,
            18 => Self::MachineCheck,
            19 => Self::SimdFloatingPoint,
            20 => Self::Virtualization,
            21 => Self::ControlProtection,
            28 => Self::HypervisorInjection,
            29 => Self::VmmCommunication,
            30 => Self::Security,
            _ => return None,
        })
    }

    /// Whether the CPU pushes an error code for this exception.
    #[must_use]
    pub const fn has_error_code(self) -> bool {
        matches!(
            self,
            Self::DoubleFault
                | Self::InvalidTss
                | Self::SegmentNotPresent
                | Self::StackSegmentFault
                | Self::GeneralProtection
                | Self::PageFault
                | Self::AlignmentCheck
                | Self::ControlProtection
                | Self::VmmCommunication
                | Self::Security
        )
    }

    /// The SDM's name for the exception.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::DivideError => "divide error",
            Self::Debug => "debug exception",
            Self::Nmi => "non-maskable interrupt",
            Self::Breakpoint => "breakpoint",
            Self::Overflow => "overflow",
            Self::BoundRange => "bound range exceeded",
            Self::InvalidOpcode => "invalid opcode",
            Self::DeviceNotAvailable => "device not available",
            Self::DoubleFault => "double fault",
            Self::InvalidTss => "invalid TSS",
            Self::SegmentNotPresent => "segment not present",
            Self::StackSegmentFault => "stack-segment fault",
            Self::GeneralProtection => "general protection fault",
            Self::PageFault => "page fault",
            Self::X87FloatingPoint => "x87 floating-point error",
            Self::AlignmentCheck => "alignment check",
            Self::MachineCheck => "machine check",
            Self::SimdFloatingPoint => "SIMD floating-point exception",
            Self::Virtualization => "virtualization exception",
            Self::ControlProtection => "control protection exception",
            Self::HypervisorInjection => "hypervisor injection exception",
            Self::VmmCommunication => "VMM communication exception",
            Self::Security => "security exception",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_round_trip() {
        for vector in 0..32 {
            if let Some(exception) = Exception::from_vector(vector) {
                assert_eq!(exception as u64, vector);
            }
        }
    }

    #[test]
    fn reserved_vectors_have_no_exception() {
        for vector in [9, 15, 22, 23, 24, 25, 26, 27, 31, 32, 255] {
            assert_eq!(Exception::from_vector(vector), None, "vector {vector}");
        }
    }

    #[test]
    fn error_code_vectors_match_the_sdm() {
        let with_code: Vec<u64> = (0..32)
            .filter(|&v| Exception::from_vector(v).is_some_and(Exception::has_error_code))
            .collect();
        assert_eq!(with_code, [8, 10, 11, 12, 13, 14, 17, 21, 29, 30]);
    }
}
