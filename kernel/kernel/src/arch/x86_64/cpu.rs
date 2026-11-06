//! CPU control instructions.

use core::arch::asm;

use arch_x86_64::registers::control::{Cr3, Cr3Flags, Cr4, Cr4Flags};

/// Masks maskable interrupts on this CPU.
pub(crate) fn disable_interrupts() {
    // SAFETY: `cli` only clears the interrupt flag. Code that later relies on
    // interrupts would hang, so only the panic path and boot call this. The
    // block may touch memory, so the compiler keeps memory accesses on their
    // side of it.
    unsafe { asm!("cli", options(nostack)) };
}

/// Stops this CPU for good. Interrupts are masked first, so `hlt` does not
/// return except for an NMI, which the loop absorbs.
pub(crate) fn halt_forever() -> ! {
    disable_interrupts();
    loop {
        // SAFETY: `hlt` stops the CPU until the next interrupt.
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

/// Raises a breakpoint exception (#BP) with `int3`.
pub(crate) fn breakpoint() {
    // SAFETY: `int3` enters the #BP handler, which returns to the next
    // instruction with every register restored.
    unsafe { asm!("int3", options(nomem, nostack)) };
}

/// `IA32_EFER`, the extended feature enable register (Vol. 4, Table 2-2).
const IA32_EFER: u32 = 0xc000_0080;

/// Reads the model-specific register `msr`.
///
/// # Safety
///
/// `msr` must exist on this CPU, or `rdmsr` raises #GP, and reading it
/// must have no side effects.
unsafe fn rdmsr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: the caller guarantees the register exists and reading it
    // changes nothing.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    u64::from(high) << 32 | u64::from(low)
}

/// Reads `IA32_EFER`.
pub(crate) fn efer() -> u64 {
    // SAFETY: `IA32_EFER` exists on every 64-bit CPU, and reading it has
    // no side effects.
    unsafe { rdmsr(IA32_EFER) }
}

/// `IA32_APIC_BASE`, the local APIC's base address and mode (Vol. 4,
/// Table 2-2).
const IA32_APIC_BASE: u32 = 0x1b;

/// Reads `IA32_APIC_BASE`.
pub(crate) fn apic_base() -> u64 {
    // SAFETY: every 64-bit CPU has a local APIC and this register, and
    // reading it has no side effects.
    unsafe { rdmsr(IA32_APIC_BASE) }
}

/// Whether the CPU supports 1 GiB pages: CPUID.80000001H:EDX.Page1GB,
/// bit 26 (Vol. 2A, CPUID).
pub(crate) fn has_1g_pages() -> bool {
    use core::arch::x86_64::__cpuid;
    const PAGE_1GB: u32 = 1 << 26;
    let max_extended = __cpuid(0x8000_0000).eax;
    max_extended >= 0x8000_0001 && __cpuid(0x8000_0001).edx & PAGE_1GB != 0
}

/// The PML4 frame CR3 points at. The cache flags are dropped.
pub(crate) fn cr3() -> mm::PhysFrame {
    mm::PhysFrame::containing(Cr3::read().0)
}

/// Loads `root` into CR3, then flushes every TLB entry, global ones
/// included.
///
/// # Safety
///
/// `root` must be a PML4 that maps the code, stack and data in use,
/// including the caller's return address and everything it references.
pub(crate) unsafe fn switch_page_tables(root: mm::PhysFrame) {
    // SAFETY: the caller guarantees the new tables map everything in use.
    unsafe { Cr3::write(root.start(), Cr3Flags::empty()) };
    let cr4 = Cr4::read();
    if cr4.contains(Cr4Flags::PAGE_GLOBAL) {
        // SAFETY: clearing CR4.PGE flushes global entries, and restoring it
        // turns global pages back on (Vol. 3A §5.10.4.1). No other bit
        // changes.
        unsafe {
            Cr4::write(cr4 - Cr4Flags::PAGE_GLOBAL);
            Cr4::write(cr4);
        }
    }
}

/// The current stack pointer.
pub(crate) fn stack_pointer() -> u64 {
    let value: u64;
    // SAFETY: reading RSP has no side effects.
    unsafe { asm!("mov {}, rsp", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Moves RSP to `top` and calls `entry(arg)`, which never returns. The old
/// stack is abandoned.
///
/// # Safety
///
/// `top` must be the 16-byte aligned top of a mapped, writable stack that
/// nothing else uses, and `arg` must be valid for `entry`.
pub(crate) unsafe fn switch_stack(top: u64, entry: extern "C" fn(u64) -> !, arg: u64) -> ! {
    // SAFETY: the caller guarantees the stack is usable. `call` pushes a
    // return address, leaving RSP at 8 mod 16 on entry as the SysV ABI
    // requires.
    unsafe {
        asm!(
            "mov rsp, {top}",
            "xor ebp, ebp",
            "call {entry}",
            "ud2",
            top = in(reg) top,
            entry = in(reg) entry,
            in("rdi") arg,
            options(noreturn),
        );
    }
}

/// RFLAGS.IF, bit 9: maskable interrupts are enabled.
const RFLAGS_IF: u64 = 1 << 9;

/// Masks interrupts and returns whether they were enabled before.
///
/// Neither this nor [`restore_interrupts`] is marked `nomem`, so the
/// compiler does not move memory accesses across them. Code between the two
/// runs with interrupts masked.
pub(crate) fn save_and_disable_interrupts() -> bool {
    let rflags: u64;
    // SAFETY: `pushfq; pop` reads RFLAGS through the stack, and `cli` only
    // clears IF. The caller restores IF with `restore_interrupts`.
    unsafe { asm!("pushfq", "pop {}", "cli", out(reg) rflags, options(preserves_flags)) };
    rflags & RFLAGS_IF != 0
}

/// Enables interrupts again if `enabled`, the value
/// [`save_and_disable_interrupts`] returned.
pub(crate) fn restore_interrupts(enabled: bool) {
    if enabled {
        // SAFETY: the caller had interrupts enabled before masking them.
        unsafe { asm!("sti", options(nostack)) };
    }
}

/// `IA32_GS_BASE`, the GS segment base (Vol. 4, Table 2-2).
const IA32_GS_BASE: u32 = 0xc000_0101;

/// Sets the GS base to `base`.
///
/// Loading a selector into GS resets the base, so nothing in the kernel
/// does.
///
/// # Safety
///
/// Code that reads through GS, such as [`read_gs`], must find what it
/// expects at `base`.
pub(crate) unsafe fn set_gs_base(base: u64) {
    #[allow(clippy::cast_possible_truncation)] // `wrmsr` takes the halves
    let (low, high) = (base as u32, (base >> 32) as u32);
    // SAFETY: `IA32_GS_BASE` exists on every 64-bit CPU, and the caller
    // vouches for the value.
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") IA32_GS_BASE,
            in("eax") low,
            in("edx") high,
            options(nostack, preserves_flags),
        );
    }
}

/// The GS base.
pub(crate) fn gs_base() -> u64 {
    // SAFETY: `IA32_GS_BASE` exists on every 64-bit CPU, and reading it
    // has no side effects.
    unsafe { rdmsr(IA32_GS_BASE) }
}

/// Reads the `u64` at `offset` from the GS base.
///
/// # Safety
///
/// The GS base plus `offset` must point at a readable, initialized `u64`.
pub(crate) unsafe fn gs_read_u64(offset: usize) -> u64 {
    let value: u64;
    // SAFETY: the caller vouches for the address. The block is not `pure`,
    // so the compiler reads GS every time instead of reusing an old value.
    unsafe {
        asm!(
            "mov {value}, gs:[{offset}]",
            offset = in(reg) offset,
            value = out(reg) value,
            options(nostack, preserves_flags, readonly),
        );
    }
    value
}

/// Reads the `u32` at `offset` from the GS base.
///
/// # Safety
///
/// As for [`gs_read_u64`], for a `u32`.
pub(crate) unsafe fn gs_read_u32(offset: usize) -> u32 {
    let value: u32;
    // SAFETY: as in `gs_read_u64`.
    unsafe {
        asm!(
            "mov {value:e}, gs:[{offset}]",
            offset = in(reg) offset,
            value = out(reg) value,
            options(nostack, preserves_flags, readonly),
        );
    }
    value
}

/// Reads the byte at `offset` from the GS base.
///
/// # Safety
///
/// As for [`gs_read_u64`], for a byte.
pub(crate) unsafe fn gs_read_u8(offset: usize) -> u8 {
    let value: u8;
    // SAFETY: as in `gs_read_u64`.
    unsafe {
        asm!(
            "mov {value}, byte ptr gs:[{offset}]",
            offset = in(reg) offset,
            value = out(reg_byte) value,
            options(nostack, preserves_flags, readonly),
        );
    }
    value
}

/// Adds 1 to the `u32` at `offset` from the GS base, in one instruction,
/// so an interrupt on this CPU cannot split the update.
///
/// # Safety
///
/// The GS base plus `offset` must point at a `u32` that only this CPU
/// writes, such as an `AtomicU32` in its per-CPU block.
pub(crate) unsafe fn gs_inc_u32(offset: usize) {
    // SAFETY: the caller vouches for the address. `inc` changes the flags.
    unsafe {
        asm!(
            "inc dword ptr gs:[{offset}]",
            offset = in(reg) offset,
            options(nostack),
        );
    }
}

/// Subtracts 1 from the `u32` at `offset` from the GS base, in one
/// instruction, and reports whether it reached 0.
///
/// # Safety
///
/// As for [`gs_inc_u32`].
pub(crate) unsafe fn gs_dec_u32(offset: usize) -> bool {
    let zero: u8;
    // SAFETY: as in `gs_inc_u32`.
    unsafe {
        asm!(
            "dec dword ptr gs:[{offset}]",
            "setz {zero}",
            offset = in(reg) offset,
            zero = out(reg_byte) zero,
            options(nostack),
        );
    }
    zero != 0
}

/// Enables maskable interrupts and halts until the next one arrives.
///
/// `sti` holds interrupts off until after the next instruction (SDM Vol.
/// 2B, STI), so an interrupt that arrives after the caller last checked
/// for work still ends the halt instead of being taken before it.
///
/// # Safety
///
/// As for [`enable_interrupts`]. No `IrqGuard` may be alive, since it
/// expects interrupts to stay masked.
pub(crate) unsafe fn enable_interrupts_and_halt() {
    // SAFETY: the caller guarantees no unhandled interrupt can arrive and
    // that nothing expects interrupts masked. `hlt` returns after the
    // interrupt's handler does.
    unsafe { asm!("sti", "hlt", options(nostack)) };
}

/// Whether maskable interrupts are enabled on this CPU.
pub(crate) fn interrupts_enabled() -> bool {
    let rflags: u64;
    // SAFETY: `pushfq; pop` reads RFLAGS through the stack and changes
    // nothing else.
    unsafe { asm!("pushfq", "pop {}", out(reg) rflags, options(nomem, preserves_flags)) };
    rflags & RFLAGS_IF != 0
}

/// Enables maskable interrupts on this CPU.
///
/// # Safety
///
/// Every interrupt that can arrive must have a handler. Until the kernel
/// handles device interrupts, the legacy PIC must be masked.
pub(crate) unsafe fn enable_interrupts() {
    // SAFETY: the caller guarantees no unhandled interrupt can arrive. The
    // block may touch memory, so memory accesses stay on their side of it.
    unsafe { asm!("sti", options(nostack)) };
}
