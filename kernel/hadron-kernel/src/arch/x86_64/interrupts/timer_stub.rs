//! Custom naked timer preemption stub for vector 254 (LAPIC timer).
//!
//! Replaces the generic `make_stub!` entry for the timer vector. When the
//! timer fires from ring 0, it performs a standard tick-and-EOI then returns
//! via `iretq`. When the timer fires from ring 3, it saves the full user
//! register state into [`USER_CONTEXT`], performs the tick-and-EOI, restores
//! the kernel address space and GS bases, then longjmps back to
//! [`process_task`](crate::proc) via `restore_kernel_context`.
//!
//! [`USER_CONTEXT`]: crate::proc::USER_CONTEXT

use crate::arch::x86_64::acpi::timer_tick_and_eoi;
use crate::proc::{KERNEL_CR3, TRAP_PREEMPTED};

/// MSR address for `IA32_GS_BASE`.
const IA32_GS_BASE_MSR: u32 = 0xC000_0101;

/// MSR address for `IA32_KERNEL_GS_BASE`.
const IA32_KERNEL_GS_BASE_MSR: u32 = 0xC000_0102;

/// Naked timer interrupt handler installed in the IDT for vector 254.
///
/// # Ring-0 path
///
/// Saves scratch registers, calls [`timer_tick_and_eoi`], restores scratch
/// registers, and returns via `iretq`. This is equivalent to the generic
/// `make_stub!` handler.
///
/// # Ring-3 path (preemption)
///
/// 1. `swapgs` to restore kernel GS base
/// 2. Saves all user GPRs + interrupt frame (RIP/RSP/RFLAGS) to `USER_CONTEXT`
/// 3. Calls [`timer_tick_and_eoi`] (tick + wake + preempt flag + LAPIC EOI)
/// 4. Restores kernel CR3 from `KERNEL_CR3`
/// 5. Fixes GS bases so both point to the per-CPU data
/// 6. Sets `TRAP_REASON = TRAP_PREEMPTED`
/// 7. Loads `SAVED_KERNEL_RSP` and pops callee-saved registers to longjmp
///    back into `process_task`
///
/// # Safety
///
/// Must only be used as an IDT entry handler. The function assumes the
/// standard x86-64 interrupt frame layout on the stack.
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn timer_preempt_stub() {
    core::arch::naked_asm!(
        // ── Check privilege level of interrupted code ──
        // CS is at [rsp+8] in the interrupt frame. RPL bits 0:1 indicate ring.
        "test qword ptr [rsp + 8], 3",
        "jnz 2f",

        // ── Ring 0: standard timer handling ──
        // Save all scratch registers (caller-saved in System V ABI).
        // 9 pushes = 72 bytes; with 40-byte interrupt frame the total
        // displacement from the pre-interrupt RSP is 112 = 16*7, keeping
        // RSP 16-byte aligned for the call.
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",

        "call {dispatch}",

        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rdi",
        "pop rsi",
        "pop rdx",
        "pop rcx",
        "pop rax",
        "iretq",

        // ── Ring 3: preempt userspace ──
        "2:",
        // GS_BASE was 0 (user), KERNEL_GS_BASE was percpu.
        // swapgs: GS_BASE = percpu, KERNEL_GS_BASE = 0.
        "swapgs",

        // Save user rax so we can use it as a scratch register.
        "push rax",

        // Load this CPU's USER_CONTEXT pointer from PerCpu.user_context_ptr
        // (GS:[32]). This is a per-CPU pointer set during init.
        "mov rax, gs:[32]",

        // Save all user GPRs into USER_CONTEXT.
        // UserRegisters layout (repr(C), each field 8 bytes):
        //   rax=0, rbx=8, rcx=16, rdx=24, rsi=32, rdi=40, rbp=48,
        //   r8=56, r9=64, r10=72, r11=80, r12=88, r13=96, r14=104,
        //   r15=112, rip=120, rsp=128, rflags=136
        "mov [rax + 8],   rbx",
        "mov [rax + 16],  rcx",
        "mov [rax + 24],  rdx",
        "mov [rax + 32],  rsi",
        "mov [rax + 40],  rdi",
        "mov [rax + 48],  rbp",
        "mov [rax + 56],  r8",
        "mov [rax + 64],  r9",
        "mov [rax + 72],  r10",
        "mov [rax + 80],  r11",
        "mov [rax + 88],  r12",
        "mov [rax + 96],  r13",
        "mov [rax + 104], r14",
        "mov [rax + 112], r15",

        // Recover original user rax from the stack and save it.
        "pop rcx",
        "mov [rax], rcx",

        // Save interrupt frame fields (RIP, RSP, RFLAGS).
        // After the pop, the stack holds the original interrupt frame:
        //   [rsp+0]  = RIP   (user instruction pointer)
        //   [rsp+8]  = CS
        //   [rsp+16] = RFLAGS
        //   [rsp+24] = RSP   (user stack pointer)
        //   [rsp+32] = SS
        "mov rcx, [rsp]",
        "mov [rax + 120], rcx",     // rip
        "mov rcx, [rsp + 24]",
        "mov [rax + 128], rcx",     // rsp
        "mov rcx, [rsp + 16]",
        "mov [rax + 136], rcx",     // rflags

        // Align the stack to 16 bytes before calling Rust code.
        // RSP = RSP0 - 40 (5 qwords from interrupt frame). If RSP0 was
        // 16-aligned, RSP is 16n+8. Subtracting 8 makes it 16-aligned.
        "sub rsp, 8",
        "call {dispatch}",
        "add rsp, 8",

        // Restore kernel CR3 from the KERNEL_CR3 static (global, not per-CPU).
        "lea rax, [rip + {kernel_cr3}]",
        "mov rax, [rax]",
        "mov cr3, rax",

        // Fix GS bases: copy GS_BASE to KERNEL_GS_BASE so both
        // point to the per-CPU data (matching normal kernel state).
        "mov ecx, {gs_base_msr}",
        "rdmsr",                    // edx:eax = GS_BASE (percpu)
        "mov ecx, {kgs_base_msr}",
        "wrmsr",                    // KERNEL_GS_BASE = percpu

        // Set TRAP_REASON = TRAP_PREEMPTED via per-CPU pointer (GS:[48]).
        "mov rax, gs:[48]",
        "mov byte ptr [rax], {preempted}",

        // Inline restore_kernel_context: load the saved kernel RSP
        // from per-CPU pointer (GS:[40]) and pop callee-saved registers
        // (matching enter_userspace_save layout), then ret back into
        // process_task.
        "mov rax, gs:[40]",
        "mov rsp, [rax]",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",

        dispatch      = sym timer_tick_and_eoi,
        kernel_cr3    = sym KERNEL_CR3,
        gs_base_msr   = const IA32_GS_BASE_MSR,
        kgs_base_msr  = const IA32_KERNEL_GS_BASE_MSR,
        preempted     = const TRAP_PREEMPTED,
    );
}
