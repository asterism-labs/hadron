//! CPU control instructions.

use core::arch::asm;

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

/// Reads CR2, which holds the address of the last page fault.
pub(crate) fn cr2() -> u64 {
    let value: u64;
    // SAFETY: reading CR2 has no side effects.
    unsafe { asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// The PML4 frame CR3 points at. The low 12 bits, which hold PCID or
/// cache flags, are dropped.
pub(crate) fn cr3() -> mm::PhysFrame {
    let value: u64;
    // SAFETY: reading CR3 has no side effects.
    unsafe { asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags)) };
    mm::PhysFrame::containing(arch_x86_64::PhysAddr::new(value & !0xfff))
}

/// Loads `root` into CR3, then flushes every TLB entry, global ones
/// included.
///
/// # Safety
///
/// `root` must be a PML4 that maps the code, stack and data in use,
/// including the caller's return address and everything it references.
pub(crate) unsafe fn switch_page_tables(root: mm::PhysFrame) {
    /// CR4.PGE, bit 7: global pages.
    const CR4_PGE: u64 = 1 << 7;
    // SAFETY: the caller guarantees the new tables map everything in use.
    // Clearing CR4.PGE flushes global entries, and restoring it turns
    // global pages back on (Vol. 3A §5.10.4.1).
    unsafe {
        asm!(
            "mov cr3, {root}",
            "mov {cr4}, cr4",
            "test {cr4}, {pge}",
            "jz 2f",
            "mov {tmp}, {cr4}",
            "xor {tmp}, {pge}",
            "mov cr4, {tmp}",
            "mov cr4, {cr4}",
            "2:",
            root = in(reg) root.start().as_u64(),
            cr4 = out(reg) _,
            tmp = out(reg) _,
            pge = in(reg) CR4_PGE,
            options(nostack),
        );
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
    let (low, high): (u32, u32);
    // SAFETY: reading `IA32_GS_BASE` has no side effects.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") IA32_GS_BASE,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    u64::from(high) << 32 | u64::from(low)
}

/// Reads the `u64` at offset 0 from the GS base.
///
/// # Safety
///
/// The GS base must point at a readable, initialized `u64`.
pub(crate) unsafe fn read_gs() -> u64 {
    let value: u64;
    // SAFETY: the caller vouches for the GS base. The block is not `pure`,
    // so the compiler reads GS every time instead of reusing an old value.
    unsafe {
        asm!(
            "mov {}, gs:[0]",
            out(reg) value,
            options(nostack, preserves_flags, readonly),
        );
    }
    value
}
