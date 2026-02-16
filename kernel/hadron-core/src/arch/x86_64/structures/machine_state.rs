//! Comprehensive CPU state snapshot for diagnostics.
//!
//! [`MachineState`] captures control registers, segment selectors, RFLAGS, and
//! EFER into a single struct for formatted display during exceptions and panics.
//! GPRs are deliberately excluded — see the type-level docs for rationale.

use core::fmt;

use crate::addr::{PhysAddr, VirtAddr};
use crate::arch::x86_64::instructions::segmentation;
use crate::arch::x86_64::registers::control::{Cr0, Cr0Flags, Cr2, Cr3, Cr4, Cr4Flags};
use crate::arch::x86_64::registers::model_specific::{EferFlags, IA32_EFER};
use crate::arch::x86_64::registers::rflags::{self, RFlags};
use crate::arch::x86_64::structures::gdt::SegmentSelector;
use crate::arch::x86_64::structures::idt::InterruptStackFrame;

/// A snapshot of CPU state for diagnostic output.
///
/// Captures everything useful that can be read from an exception handler or
/// panic handler: instruction/stack pointers, segment selectors, control
/// registers, RFLAGS, and EFER.
///
/// General-purpose registers (RAX-R15) are **not** included because:
/// - In `x86-interrupt` handlers, the ABI clobbers GPRs before handler code runs.
/// - In `capture()`, GPRs would reflect this function's own prologue, not the caller.
/// Faithful GPR capture requires a naked assembly trampoline (future work).
#[derive(Debug, Clone, Copy)]
pub struct MachineState {
    /// Instruction pointer.
    pub rip: VirtAddr,
    /// Stack pointer.
    pub rsp: VirtAddr,
    /// Code segment selector.
    pub cs: SegmentSelector,
    /// Data segment selector.
    pub ds: SegmentSelector,
    /// Extra segment selector.
    pub es: SegmentSelector,
    /// FS segment selector.
    pub fs: SegmentSelector,
    /// GS segment selector.
    pub gs: SegmentSelector,
    /// Stack segment selector.
    pub ss: SegmentSelector,
    /// CR0 flags.
    pub cr0: Cr0Flags,
    /// CR2 — page fault linear address. Stored as raw `u64` because the
    /// faulting address may be non-canonical.
    pub cr2: u64,
    /// CR3 — page table root physical address.
    pub cr3: PhysAddr,
    /// CR4 flags.
    pub cr4: Cr4Flags,
    /// RFLAGS register.
    pub rflags: RFlags,
    /// IA32_EFER model-specific register.
    pub efer: EferFlags,
}

impl MachineState {
    /// Creates a `MachineState` from a hardware-pushed interrupt stack frame.
    ///
    /// RIP, CS, RFLAGS, RSP, and SS come from the frame (as pushed by the CPU).
    /// All other fields are read from the current CPU state.
    pub fn from_interrupt_frame(frame: &InterruptStackFrame) -> Self {
        Self {
            rip: frame.instruction_pointer,
            rsp: frame.stack_pointer,
            cs: SegmentSelector::from_raw(frame.code_segment as u16),
            ss: SegmentSelector::from_raw(frame.stack_segment as u16),
            ds: segmentation::read_ds(),
            es: segmentation::read_es(),
            fs: segmentation::read_fs(),
            gs: segmentation::read_gs(),
            cr0: Cr0::read(),
            cr2: Cr2::read(),
            cr3: Cr3::read(),
            cr4: Cr4::read(),
            rflags: RFlags::from_bits_truncate(frame.cpu_flags),
            efer: EferFlags::from_bits_truncate(unsafe { IA32_EFER.read() }),
        }
    }

    /// Captures the current CPU state.
    ///
    /// RIP and RSP are approximate — they reflect this call site (when inlined)
    /// rather than exact values at an arbitrary point. Call this as early as
    /// possible (e.g. first statement in a panic handler) for the most useful
    /// values.
    #[inline(always)]
    pub fn capture() -> Self {
        let rip: u64;
        let rsp: u64;
        unsafe {
            core::arch::asm!(
                "lea {}, [rip]",
                out(reg) rip,
                options(nomem, nostack, preserves_flags),
            );
            core::arch::asm!(
                "mov {}, rsp",
                out(reg) rsp,
                options(nomem, nostack, preserves_flags),
            );
        }

        Self {
            rip: VirtAddr::new_truncate(rip),
            rsp: VirtAddr::new_truncate(rsp),
            cs: segmentation::read_cs(),
            ds: segmentation::read_ds(),
            es: segmentation::read_es(),
            fs: segmentation::read_fs(),
            gs: segmentation::read_gs(),
            ss: segmentation::read_ss(),
            cr0: Cr0::read(),
            cr2: Cr2::read(),
            cr3: Cr3::read(),
            cr4: Cr4::read(),
            rflags: rflags::read(),
            efer: EferFlags::from_bits_truncate(unsafe { IA32_EFER.read() }),
        }
    }
}

/// Formats a segment selector as `0xNNNN (idx=N, rpl=N)`.
fn fmt_selector(f: &mut fmt::Formatter<'_>, sel: SegmentSelector) -> fmt::Result {
    write!(
        f,
        "{:#06x} (idx={}, rpl={})",
        sel.as_u16(),
        sel.index(),
        sel.rpl()
    )
}

impl fmt::Display for MachineState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "--- Machine State ---")?;

        // RIP / RSP
        writeln!(
            f,
            "  RIP: {:#018x}    RSP: {:#018x}",
            self.rip.as_u64(),
            self.rsp.as_u64()
        )?;

        // CS / SS
        write!(f, "  CS:  ")?;
        fmt_selector(f, self.cs)?;
        write!(f, "    SS:  ")?;
        fmt_selector(f, self.ss)?;
        writeln!(f)?;

        // DS / ES
        write!(f, "  DS:  ")?;
        fmt_selector(f, self.ds)?;
        write!(f, "    ES:  ")?;
        fmt_selector(f, self.es)?;
        writeln!(f)?;

        // FS / GS
        write!(f, "  FS:  ")?;
        fmt_selector(f, self.fs)?;
        write!(f, "    GS:  ")?;
        fmt_selector(f, self.gs)?;
        writeln!(f)?;

        // CR0
        writeln!(
            f,
            "  CR0: {:#018x}  {:?}",
            self.cr0.bits(),
            self.cr0
        )?;

        // CR2
        writeln!(f, "  CR2: {:#018x}", self.cr2)?;

        // CR3
        writeln!(f, "  CR3: {:#018x}", self.cr3.as_u64())?;

        // CR4
        writeln!(
            f,
            "  CR4: {:#018x}  {:?}",
            self.cr4.bits(),
            self.cr4
        )?;

        // RFLAGS
        writeln!(
            f,
            "  RFLAGS: {:#018x}  {:?}",
            self.rflags.bits(),
            self.rflags
        )?;

        // EFER
        write!(
            f,
            "  EFER:   {:#018x}  {:?}",
            self.efer.bits(),
            self.efer
        )
    }
}
