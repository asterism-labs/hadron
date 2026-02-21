//! Custom naked timer preemption stub for vector 254 (LAPIC timer).
//!
//! Replaces the generic `make_stub!` entry for the timer vector. When the
//! timer fires from ring 0, it performs a standard tick-and-EOI then returns
//! via `iretq`. When the timer fires from ring 3, it saves the full user
//! register state into [`USER_CONTEXT`], performs the tick-and-EOI, restores
//! the kernel address space and GS bases, then longjmps back to
//! [`process_task`](crate::proc) via `restore_kernel_context`.
//!
//! When `hadron_profile_sample` is enabled, the ring-0 path additionally
//! captures the interrupted RIP, RSP, and RBP and calls into the sampling
//! profiler before the timer dispatch.
//!
//! [`USER_CONTEXT`]: crate::proc::USER_CONTEXT

use crate::arch::x86_64::acpi::timer_tick_and_eoi;
use crate::proc::{KERNEL_CR3, TRAP_PREEMPTED};

/// MSR address for `IA32_GS_BASE`.
const IA32_GS_BASE_MSR: u32 = 0xC000_0101;

/// MSR address for `IA32_KERNEL_GS_BASE`.
const IA32_KERNEL_GS_BASE_MSR: u32 = 0xC000_0102;

// ---------------------------------------------------------------------------
// Standard timer stub (no profiling)
// ---------------------------------------------------------------------------

/// Naked timer interrupt handler (standard, non-profiling variant).
#[cfg(not(hadron_profile_sample))]
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
        "swapgs",
        "push rax",
        "mov rax, gs:[32]",
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
        "pop rcx",
        "mov [rax], rcx",
        "mov rcx, [rsp]",
        "mov [rax + 120], rcx",
        "mov rcx, [rsp + 24]",
        "mov [rax + 128], rcx",
        "mov rcx, [rsp + 16]",
        "mov [rax + 136], rcx",
        "sub rsp, 8",
        "call {dispatch}",
        "add rsp, 8",
        "lea rax, [rip + {kernel_cr3}]",
        "mov rax, [rax]",
        "mov cr3, rax",
        "mov ecx, {gs_base_msr}",
        "rdmsr",
        "mov ecx, {kgs_base_msr}",
        "wrmsr",
        "mov rax, gs:[48]",
        "mov byte ptr [rax], {preempted}",
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

// ---------------------------------------------------------------------------
// Profiling timer stub (with sampling hook)
// ---------------------------------------------------------------------------

#[cfg(hadron_profile_sample)]
use crate::profiling::sample::sample_capture;

/// Naked timer interrupt handler (profiling variant with sampling hook).
///
/// The ring-0 path captures the interrupted RIP, RSP, and RBP from the
/// interrupt frame and calls [`sample_capture`] before the timer dispatch.
/// This adds ~3 instructions + 1 call to the hot path. When the profiler
/// is inactive, `sample_capture` returns after a single atomic load.
#[cfg(hadron_profile_sample)]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn timer_preempt_stub() {
    core::arch::naked_asm!(
        // ── Check privilege level of interrupted code ──
        "test qword ptr [rsp + 8], 3",
        "jnz 2f",

        // ── Ring 0: timer handling with sampling hook ──
        // Save all scratch registers.
        // 9 pushes = 72 bytes; RSP is 16-byte aligned.
        "push rax",
        "push rcx",
        "push rdx",
        "push rsi",
        "push rdi",
        "push r8",
        "push r9",
        "push r10",
        "push r11",

        // ── Sampling profiler hook ──
        // Pass interrupted context to sample_capture(rip, rsp, rbp).
        // After 9 pushes (72B), the interrupt frame starts at [rsp+72]:
        //   [rsp+72] = RIP, [rsp+80] = CS, [rsp+88] = RFLAGS,
        //   [rsp+96] = RSP, [rsp+104] = SS
        // RBP is callee-saved and still holds its interrupted value.
        "mov rdi, [rsp + 72]",   // arg0: interrupted RIP
        "mov rsi, [rsp + 96]",   // arg1: interrupted RSP
        "mov rdx, rbp",          // arg2: interrupted RBP (frame pointer)
        "call {sample_capture}",

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

        // ── Ring 3: preempt userspace (identical to non-profiling variant) ──
        "2:",
        "swapgs",
        "push rax",
        "mov rax, gs:[32]",
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
        "pop rcx",
        "mov [rax], rcx",
        "mov rcx, [rsp]",
        "mov [rax + 120], rcx",
        "mov rcx, [rsp + 24]",
        "mov [rax + 128], rcx",
        "mov rcx, [rsp + 16]",
        "mov [rax + 136], rcx",
        "sub rsp, 8",
        "call {dispatch}",
        "add rsp, 8",
        "lea rax, [rip + {kernel_cr3}]",
        "mov rax, [rax]",
        "mov cr3, rax",
        "mov ecx, {gs_base_msr}",
        "rdmsr",
        "mov ecx, {kgs_base_msr}",
        "wrmsr",
        "mov rax, gs:[48]",
        "mov byte ptr [rax], {preempted}",
        "mov rax, gs:[40]",
        "mov rsp, [rax]",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",

        sample_capture = sym sample_capture,
        dispatch       = sym timer_tick_and_eoi,
        kernel_cr3     = sym KERNEL_CR3,
        gs_base_msr    = const IA32_GS_BASE_MSR,
        kgs_base_msr   = const IA32_KERNEL_GS_BASE_MSR,
        preempted      = const TRAP_PREEMPTED,
    );
}
