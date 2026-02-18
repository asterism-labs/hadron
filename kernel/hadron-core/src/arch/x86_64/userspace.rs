//! Userspace entry primitives for x86_64.
//!
//! Provides [`jump_to_userspace`] for the initial transition to ring 3 via
//! `iretq`, [`enter_userspace_save`] for a setjmp-style entry that can be
//! "returned from" via [`restore_kernel_context`], and [`UserRegisters`] for
//! saving/restoring user-mode state.

/// Saved user-mode register state.
///
/// Stored when entering the kernel via SYSCALL or interrupt, and restored
/// when returning to user mode.
#[allow(dead_code, reason = "reserved for Phase 9 preemption")]
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct UserRegisters {
    /// General-purpose registers.
    pub rax: u64,
    /// RBX register.
    pub rbx: u64,
    /// RCX register (clobbered by SYSCALL — holds return RIP).
    pub rcx: u64,
    /// RDX register.
    pub rdx: u64,
    /// RSI register.
    pub rsi: u64,
    /// RDI register.
    pub rdi: u64,
    /// RBP register.
    pub rbp: u64,
    /// R8 register.
    pub r8: u64,
    /// R9 register.
    pub r9: u64,
    /// R10 register.
    pub r10: u64,
    /// R11 register (clobbered by SYSCALL — holds return RFLAGS).
    pub r11: u64,
    /// R12 register.
    pub r12: u64,
    /// R13 register.
    pub r13: u64,
    /// R14 register.
    pub r14: u64,
    /// R15 register.
    pub r15: u64,
    /// User instruction pointer.
    pub rip: u64,
    /// User stack pointer.
    pub rsp: u64,
    /// User RFLAGS.
    pub rflags: u64,
}

/// GDT selector for user data segment: index 3, RPL=3.
///
/// GDT layout: null(0), kernel_code(0x08), kernel_data(0x10),
/// user_data(0x18), user_code(0x20).
/// SS = 0x18 | 3 = 0x1B.
pub const USER_DATA_SELECTOR: u64 = 0x1B;

/// GDT selector for user code segment: index 4, RPL=3.
///
/// CS = 0x20 | 3 = 0x23.
pub const USER_CODE_SELECTOR: u64 = 0x23;

/// Initial RFLAGS for user mode: IF=1 (interrupts enabled), reserved bit 1 set.
pub const USER_RFLAGS: u64 = 0x202;

/// Performs the initial transition to ring 3 via `iretq`.
///
/// This function never returns. It pushes an iret frame with user-mode
/// selectors and jumps to the given entry point with the given stack.
///
/// All general-purpose registers are zeroed before entry to prevent
/// information leaks from kernel state.
///
/// # Safety
///
/// - `entry` must be a valid user-mode instruction pointer mapped in
///   the current address space with USER permissions.
/// - `user_rsp` must point to a valid user-mode stack.
/// - The GDT must have user_data at index 3 and user_code at index 4.
/// - CR3 must already be loaded with the user address space.
#[unsafe(naked)]
pub unsafe extern "C" fn jump_to_userspace(entry: u64, user_rsp: u64) -> ! {
    core::arch::naked_asm!(
        // Build iretq frame on kernel stack.
        "push {user_ds}",   // SS = user_data | RPL=3
        "push rsi",         // RSP = user stack pointer
        "push {rflags}",    // RFLAGS (IF=1)
        "push {user_cs}",   // CS = user_code | RPL=3
        "push rdi",         // RIP = entry point

        // Zero all GPRs to prevent kernel info leaks.
        "xor rax, rax",
        "xor rbx, rbx",
        "xor rcx, rcx",
        "xor rdx, rdx",
        "xor rsi, rsi",
        "xor rdi, rdi",
        "xor rbp, rbp",
        "xor r8, r8",
        "xor r9, r9",
        "xor r10, r10",
        "xor r11, r11",
        "xor r12, r12",
        "xor r13, r13",
        "xor r14, r14",
        "xor r15, r15",

        // Transition to ring 3.
        "iretq",

        user_ds = const USER_DATA_SELECTOR,
        user_cs = const USER_CODE_SELECTOR,
        rflags = const USER_RFLAGS,
    );
}

/// Enters userspace via `iretq` while saving the kernel context so that
/// [`restore_kernel_context`] can "return" back to the caller.
///
/// This is the setjmp half of the setjmp/longjmp pair. It:
/// 1. Pushes callee-saved registers (rbp, rbx, r12-r15) on the current stack
/// 2. Saves RSP to `*saved_rsp_ptr`
/// 3. Builds an `iretq` frame and transitions to ring 3
///
/// When a syscall handler or fault handler calls [`restore_kernel_context`]
/// with the saved RSP, execution "returns" from this function back to the
/// caller (the process task in the executor).
///
/// # Safety
///
/// - `entry` must be a valid user-mode instruction pointer.
/// - `user_rsp` must point to a valid user-mode stack.
/// - `saved_rsp_ptr` must point to a valid `u64` for storing the kernel RSP.
/// - The GDT must have user segments at the expected indices.
/// - CR3 must already be loaded with the user address space.
/// - Interrupts must be disabled.
#[unsafe(naked)]
pub unsafe extern "C" fn enter_userspace_save(entry: u64, user_rsp: u64, saved_rsp_ptr: *mut u64) {
    core::arch::naked_asm!(
        // Save callee-saved registers so restore_kernel_context can pop them.
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Save RSP to *saved_rsp_ptr (rdx = 3rd argument).
        "mov [rdx], rsp",

        // Build iretq frame on the current stack.
        "push {user_ds}",   // SS
        "push rsi",         // RSP = user stack pointer
        "push {rflags}",    // RFLAGS (IF=1)
        "push {user_cs}",   // CS
        "push rdi",         // RIP = entry point

        // Zero GPRs to prevent kernel info leaks.
        "xor rax, rax",
        "xor rbx, rbx",
        "xor rcx, rcx",
        "xor rdx, rdx",
        "xor rsi, rsi",
        "xor rdi, rdi",
        "xor rbp, rbp",
        "xor r8, r8",
        "xor r9, r9",
        "xor r10, r10",
        "xor r11, r11",
        "xor r12, r12",
        "xor r13, r13",
        "xor r14, r14",
        "xor r15, r15",

        // Transition to ring 3.
        "iretq",

        user_ds = const USER_DATA_SELECTOR,
        user_cs = const USER_CODE_SELECTOR,
        rflags = const USER_RFLAGS,
    );
}

/// Restores the kernel context saved by [`enter_userspace_save`].
///
/// This is the longjmp half. It loads the saved RSP, pops callee-saved
/// registers, and `ret`s — effectively "returning" from `enter_userspace_save`
/// back into the process task.
///
/// # Safety
///
/// - `saved_rsp` must be the value written by `enter_userspace_save`.
/// - Must be called from kernel mode (ring 0) with a valid kernel CR3.
/// - The kernel stack pointed to by `saved_rsp` must still be intact.
#[unsafe(naked)]
pub unsafe extern "C" fn restore_kernel_context(saved_rsp: u64) -> ! {
    core::arch::naked_asm!(
        // Switch to the saved kernel stack.
        "mov rsp, rdi",
        // Pop callee-saved registers (reverse order of enter_userspace_save).
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        // Return to the caller of enter_userspace_save.
        "ret",
    );
}
