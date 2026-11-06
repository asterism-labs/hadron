//! The control registers CR0, CR2, CR3, CR4 and CR8 (SDM Vol. 3A §2.5).
//!
//! `read` keeps every bit, including bits the flag type does not name, so
//! `Debug` shows them. `write` keeps the current value of the bits the flag
//! type does not name and ignores them in its argument. The SDM requires
//! reserved CR0 bits to be preserved, and setting a reserved CR4 bit raises
//! #GP.

use bitflags::bitflags;

use crate::addr::PhysAddr;

bitflags! {
    /// CR0: operating mode and cache control.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Cr0Flags: u64 {
        /// PE, bit 0: protected mode.
        const PROTECTION_ENABLE = 1 << 0;
        /// MP, bit 1: `wait` faults when TS is set.
        const MONITOR_COPROCESSOR = 1 << 1;
        /// EM, bit 2: x87 instructions fault. Must be clear for SSE.
        const EMULATION = 1 << 2;
        /// TS, bit 3: the next x87 or SSE instruction faults with #NM.
        const TASK_SWITCHED = 1 << 3;
        /// ET, bit 4: hardwired to 1.
        const EXTENSION_TYPE = 1 << 4;
        /// NE, bit 5: x87 errors raise #MF instead of an external interrupt.
        const NUMERIC_ERROR = 1 << 5;
        /// WP, bit 16: ring 0 cannot write read-only pages.
        const WRITE_PROTECT = 1 << 16;
        /// AM, bit 18: RFLAGS.AC turns on alignment checks in ring 3.
        const ALIGNMENT_MASK = 1 << 18;
        /// NW, bit 29: not write-through.
        const NOT_WRITE_THROUGH = 1 << 29;
        /// CD, bit 30: the caches are off.
        const CACHE_DISABLE = 1 << 30;
        /// PG, bit 31: paging.
        const PAGING = 1 << 31;
    }
}

bitflags! {
    /// The flag bits of CR3 while CR4.PCIDE is clear.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Cr3Flags: u64 {
        /// PWT, bit 3: the PML4 is write-through.
        const WRITE_THROUGH = 1 << 3;
        /// PCD, bit 4: the PML4 is not cached.
        const CACHE_DISABLE = 1 << 4;
        /// Bit 61: linear address masking for 57-bit user pointers.
        const LAM_U57 = 1 << 61;
        /// Bit 62: linear address masking for 48-bit user pointers.
        const LAM_U48 = 1 << 62;
    }
}

bitflags! {
    /// CR4: architectural extensions.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct Cr4Flags: u64 {
        /// VME, bit 0: virtual-8086 mode extensions.
        const VIRTUAL_8086_EXTENSIONS = 1 << 0;
        /// PVI, bit 1: protected-mode virtual interrupts.
        const PROTECTED_VIRTUAL_INTERRUPTS = 1 << 1;
        /// TSD, bit 2: `rdtsc` faults outside ring 0.
        const TIMESTAMP_DISABLE = 1 << 2;
        /// DE, bit 3: debug extensions.
        const DEBUGGING_EXTENSIONS = 1 << 3;
        /// PSE, bit 4: 4 MiB pages in 32-bit paging.
        const PAGE_SIZE_EXTENSION = 1 << 4;
        /// PAE, bit 5: physical address extension. Required in long mode.
        const PHYSICAL_ADDRESS_EXTENSION = 1 << 5;
        /// MCE, bit 6: machine-check exceptions.
        const MACHINE_CHECK = 1 << 6;
        /// PGE, bit 7: global pages.
        const PAGE_GLOBAL = 1 << 7;
        /// PCE, bit 8: `rdpmc` works outside ring 0.
        const PERFORMANCE_COUNTER = 1 << 8;
        /// OSFXSR, bit 9: the OS uses `fxsave` and `fxrstor`, which turns on
        /// SSE.
        const OSFXSR = 1 << 9;
        /// OSXMMEXCPT, bit 10: unmasked SSE exceptions raise #XM.
        const OSXMMEXCPT = 1 << 10;
        /// UMIP, bit 11: `sgdt`, `sidt`, `sldt`, `smsw` and `str` fault
        /// outside ring 0.
        const USER_MODE_INSTRUCTION_PREVENTION = 1 << 11;
        /// LA57, bit 12: 5-level paging.
        const LA57 = 1 << 12;
        /// VMXE, bit 13: VMX.
        const VMX_ENABLE = 1 << 13;
        /// SMXE, bit 14: SMX.
        const SMX_ENABLE = 1 << 14;
        /// FSGSBASE, bit 16: `rdfsbase`, `wrfsbase`, `rdgsbase` and
        /// `wrgsbase`.
        const FSGSBASE = 1 << 16;
        /// PCIDE, bit 17: process-context identifiers in CR3.
        const PCID = 1 << 17;
        /// OSXSAVE, bit 18: XSAVE and extended states.
        const OSXSAVE = 1 << 18;
        /// KL, bit 19: Key Locker.
        const KEY_LOCKER = 1 << 19;
        /// SMEP, bit 20: ring 0 cannot execute user pages.
        const SMEP = 1 << 20;
        /// SMAP, bit 21: ring 0 cannot access user pages unless RFLAGS.AC is
        /// set.
        const SMAP = 1 << 21;
        /// PKE, bit 22: protection keys for user pages.
        const PROTECTION_KEYS_USER = 1 << 22;
        /// CET, bit 23: control-flow enforcement.
        const CONTROL_FLOW_ENFORCEMENT = 1 << 23;
        /// PKS, bit 24: protection keys for supervisor pages.
        const PROTECTION_KEYS_SUPERVISOR = 1 << 24;
        /// UINTR, bit 25: user interrupts.
        const USER_INTERRUPTS = 1 << 25;
        /// LASS, bit 27: linear-address-space separation.
        const LASS = 1 << 27;
        /// `LAM_SUP`, bit 28: linear address masking for ring 0 pointers.
        const LAM_SUPERVISOR = 1 << 28;
        /// FRED, bit 32: flexible return and event delivery.
        const FRED = 1 << 32;
    }
}

/// The PML4 address bits of CR3: 12 to 51.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
const CR3_ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

/// The largest task priority CR8 holds.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
const MAX_PRIORITY: u8 = 15;

/// CR0.
#[derive(Debug)]
pub struct Cr0;

/// CR2, the linear address of the last page fault.
#[derive(Debug)]
pub struct Cr2;

/// CR3, the root of the page tables.
#[derive(Debug)]
pub struct Cr3;

/// CR4.
#[derive(Debug)]
pub struct Cr4;

/// CR8, the task priority register.
#[derive(Debug)]
pub struct Cr8;

/// `old` with the bits in `known` replaced by those of `new`.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
const fn merge(old: u64, new: u64, known: u64) -> u64 {
    old & !known | new & known
}

/// Splits a CR3 value into the PML4 address and the flags.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
const fn split_cr3(value: u64) -> (PhysAddr, Cr3Flags) {
    (
        PhysAddr::new(value & CR3_ADDR_MASK),
        Cr3Flags::from_bits_retain(value & !CR3_ADDR_MASK),
    )
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::arch::asm;

    use super::{
        Cr0, Cr0Flags, Cr2, Cr3, Cr3Flags, Cr4, Cr4Flags, Cr8, MAX_PRIORITY, PhysAddr, merge,
        split_cr3,
    };

    /// Reads a control register.
    macro_rules! read_cr {
        ($reg:literal) => {{
            let value: u64;
            // SAFETY: reading a control register has no side effects.
            unsafe {
                asm!(
                    concat!("mov {}, ", $reg),
                    out(reg) value,
                    options(nomem, nostack, preserves_flags),
                );
            }
            value
        }};
    }

    /// Writes a control register. The block may touch memory, so the
    /// compiler keeps memory accesses on their side of it.
    macro_rules! write_cr {
        ($reg:literal, $value:expr) => {
            asm!(
                concat!("mov ", $reg, ", {}"),
                in(reg) $value,
                options(nostack, preserves_flags),
            )
        };
    }

    impl Cr0 {
        /// Reads CR0.
        #[must_use]
        pub fn read() -> Cr0Flags {
            Cr0Flags::from_bits_retain(read_cr!("cr0"))
        }

        /// Writes `flags` to CR0, keeping the bits [`Cr0Flags`] does not
        /// name.
        ///
        /// # Safety
        ///
        /// The new mode must keep the running code valid. Clearing PG or PE
        /// leaves long mode, and clearing WP lets ring 0 write read-only
        /// pages.
        pub unsafe fn write(flags: Cr0Flags) {
            let value = merge(read_cr!("cr0"), flags.bits(), Cr0Flags::all().bits());
            // SAFETY: the caller vouches for the new mode.
            unsafe { write_cr!("cr0", value) };
        }
    }

    impl Cr2 {
        /// Reads CR2. The address is a plain number, since software can
        /// write any value to CR2.
        #[must_use]
        pub fn read() -> u64 {
            read_cr!("cr2")
        }
    }

    impl Cr3 {
        /// Reads the PML4 address and the flags. CR4.PCIDE must be clear,
        /// or the flags are bits of a PCID.
        #[must_use]
        pub fn read() -> (PhysAddr, Cr3Flags) {
            split_cr3(read_cr!("cr3"))
        }

        /// Loads `root` and `flags` into CR3. This flushes every TLB entry
        /// except global ones.
        ///
        /// # Safety
        ///
        /// `root` must be a PML4 that maps the code, stack and data in use,
        /// including the caller's return address and everything it
        /// references. CR4.PCIDE must be clear.
        ///
        /// # Panics
        ///
        /// Panics if `root` is not 4 KiB aligned.
        pub unsafe fn write(root: PhysAddr, flags: Cr3Flags) {
            assert!(root.is_aligned(4096), "the PML4 is 4 KiB aligned");
            // SAFETY: the caller vouches for the tables.
            unsafe { write_cr!("cr3", root.as_u64() | flags.bits()) };
        }
    }

    impl Cr4 {
        /// Reads CR4.
        #[must_use]
        pub fn read() -> Cr4Flags {
            Cr4Flags::from_bits_retain(read_cr!("cr4"))
        }

        /// Writes `flags` to CR4, keeping the bits [`Cr4Flags`] does not
        /// name.
        ///
        /// # Safety
        ///
        /// The CPU must support every flag set, or the write raises #GP.
        /// The new mode must keep the running code valid. For example,
        /// SMAP faults on ring 0 accesses to user pages that are not
        /// wrapped in `stac` and `clac`.
        pub unsafe fn write(flags: Cr4Flags) {
            let value = merge(read_cr!("cr4"), flags.bits(), Cr4Flags::all().bits());
            // SAFETY: the caller vouches for the new mode.
            unsafe { write_cr!("cr4", value) };
        }
    }

    impl Cr8 {
        /// Reads the task priority, 0 to 15.
        #[must_use]
        #[allow(clippy::cast_possible_truncation)] // bits 63:4 are reserved
        pub fn read() -> u8 {
            read_cr!("cr8") as u8
        }

        /// Sets the task priority. The local APIC holds back interrupts
        /// whose priority class, the vector's high nibble, is `priority` or
        /// lower.
        ///
        /// # Safety
        ///
        /// Lowering the priority can let in interrupts that the running
        /// code relies on being held back.
        ///
        /// # Panics
        ///
        /// Panics if `priority` is above 15.
        pub unsafe fn write(priority: u8) {
            assert!(priority <= MAX_PRIORITY, "CR8 holds a priority of 0 to 15");
            // SAFETY: the caller vouches for the interrupts it lets in.
            unsafe { write_cr!("cr8", u64::from(priority)) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cr0_bits_match_the_sdm() {
        assert_eq!(Cr0Flags::PROTECTION_ENABLE.bits(), 0x1);
        assert_eq!(Cr0Flags::WRITE_PROTECT.bits(), 0x1_0000);
        assert_eq!(Cr0Flags::PAGING.bits(), 0x8000_0000);
    }

    #[test]
    fn cr4_bits_match_the_sdm() {
        assert_eq!(Cr4Flags::PHYSICAL_ADDRESS_EXTENSION.bits(), 0x20);
        assert_eq!(Cr4Flags::PAGE_GLOBAL.bits(), 0x80);
        assert_eq!(Cr4Flags::LA57.bits(), 0x1000);
        assert_eq!(Cr4Flags::SMAP.bits(), 0x20_0000);
        assert_eq!(Cr4Flags::FRED.bits(), 0x1_0000_0000);
    }

    #[test]
    fn write_keeps_bits_the_flags_do_not_name() {
        let reserved = 1 << 15;
        let old = reserved | Cr4Flags::PAGE_GLOBAL.bits();
        let new = Cr4Flags::PHYSICAL_ADDRESS_EXTENSION.bits() | 1 << 26;
        assert_eq!(
            merge(old, new, Cr4Flags::all().bits()),
            reserved | Cr4Flags::PHYSICAL_ADDRESS_EXTENSION.bits()
        );
    }

    #[test]
    fn cr3_splits_into_address_and_flags() {
        let (root, flags) = split_cr3(0x0000_0001_2345_6000 | 1 << 3 | 1 << 62);
        assert_eq!(root, PhysAddr::new(0x1_2345_6000));
        assert_eq!(flags, Cr3Flags::WRITE_THROUGH | Cr3Flags::LAM_U48);
    }

    #[test]
    fn cr3_keeps_unnamed_low_bits_as_flags() {
        let (root, flags) = split_cr3(0x1000 | 0x5);
        assert_eq!(root, PhysAddr::new(0x1000));
        assert_eq!(flags.bits(), 0x5);
    }
}
