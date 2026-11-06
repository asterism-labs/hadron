//! The segment registers CS, SS, DS, ES, FS and GS (SDM Vol. 3A §3.4.3).
//!
//! In 64-bit mode the CPU ignores the base and limit of CS, SS, DS and ES.
//! FS and GS keep a base, which loading a selector overwrites with the
//! descriptor's 32-bit base. Kernels set those bases through MSRs instead.

#[cfg(target_arch = "x86_64")]
use core::arch::asm;

#[cfg(target_arch = "x86_64")]
use crate::structures::segment::SegmentSelector;

/// The code segment register.
#[derive(Debug)]
pub struct Cs;

/// The stack segment register.
#[derive(Debug)]
pub struct Ss;

/// The DS data segment register.
#[derive(Debug)]
pub struct Ds;

/// The ES data segment register.
#[derive(Debug)]
pub struct Es;

/// The FS data segment register.
#[derive(Debug)]
pub struct Fs;

/// The GS data segment register.
#[derive(Debug)]
pub struct Gs;

/// Reads the selector in a segment register.
#[cfg(target_arch = "x86_64")]
macro_rules! read_segment {
    ($reg:literal) => {{
        let bits: u16;
        // SAFETY: reading a segment register has no side effects.
        unsafe {
            asm!(
                concat!("mov {0:x}, ", $reg),
                out(reg) bits,
                options(nomem, nostack, preserves_flags),
            );
        }
        SegmentSelector::from_bits(bits)
    }};
}

/// `read` and `write` for a register that `mov` can load.
#[cfg(target_arch = "x86_64")]
macro_rules! mov_segment {
    ($ty:ident, $reg:literal, $safety:literal) => {
        impl $ty {
            #[doc = concat!("Reads ", $reg, ".")]
            #[must_use]
            pub fn read() -> SegmentSelector {
                read_segment!($reg)
            }

            #[doc = concat!("Loads `selector` into ", $reg, ".")]
            ///
            /// # Safety
            ///
            #[doc = $safety]
            pub unsafe fn write(selector: SegmentSelector) {
                // SAFETY: the caller vouches for the selector. The block may
                // touch memory, so memory accesses stay on their side of it.
                unsafe {
                    asm!(
                        concat!("mov ", $reg, ", {0:x}"),
                        in(reg) selector.bits(),
                        options(nostack, preserves_flags),
                    );
                }
            }
        }
    };
}

#[cfg(target_arch = "x86_64")]
mov_segment!(
    Ss,
    "ss",
    "`selector` must name a present, writable data segment whose DPL is the \
     current privilege level, in the loaded GDT or LDT. Ring 0 may also \
     load a null selector."
);

#[cfg(target_arch = "x86_64")]
mov_segment!(
    Ds,
    "ds",
    "`selector` must be null or name a present data or readable code \
     segment in the loaded GDT or LDT that the current privilege level may \
     access."
);

#[cfg(target_arch = "x86_64")]
mov_segment!(Es, "es", "As for [`Ds::write`].");

#[cfg(target_arch = "x86_64")]
mov_segment!(
    Fs,
    "fs",
    "As for [`Ds::write`]. The load replaces the FS base, so code that \
     reads through FS must find what it expects at the new base."
);

#[cfg(target_arch = "x86_64")]
mov_segment!(
    Gs,
    "gs",
    "As for [`Ds::write`]. The load replaces the GS base, so code that \
     reads through GS, such as per-CPU data, must find what it expects at \
     the new base."
);

#[cfg(target_arch = "x86_64")]
impl Cs {
    /// Reads CS.
    #[must_use]
    pub fn read() -> SegmentSelector {
        read_segment!("cs")
    }

    /// Loads `selector` into CS through a far return.
    ///
    /// # Safety
    ///
    /// `selector` must name a present 64-bit code segment in the loaded GDT
    /// or LDT, at the current privilege level.
    pub unsafe fn write(selector: SegmentSelector) {
        // SAFETY: the caller vouches for the selector. The far return pops
        // the two values pushed here and lands on the next instruction.
        unsafe {
            asm!(
                "push {selector}",
                "lea {target}, [rip + 2f]",
                "push {target}",
                "retfq",
                "2:",
                selector = in(reg) u64::from(selector.bits()),
                target = lateout(reg) _,
                options(preserves_flags),
            );
        }
    }
}
