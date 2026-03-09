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

/// Re-enters userspace from a saved [`UserRegisters`] context.
///
/// Like [`enter_userspace_save`], this saves callee-saved registers and
/// the kernel RSP so that [`restore_kernel_context`] can "return" here.
/// Then it builds an `iretq` frame from the saved user registers and
/// restores all GPRs before transitioning to ring 3.
///
/// Used when re-entering a preempted process whose state was saved by
/// the timer preemption stub.
///
/// # Safety
///
/// - `ctx` must point to a valid [`UserRegisters`] with ring-3 RIP/RSP.
/// - `saved_rsp_ptr` must point to a valid `u64` for storing the kernel RSP.
/// - The GDT must have user segments at the expected indices.
/// - CR3 must already be loaded with the user address space.
/// - Interrupts must be disabled.
#[unsafe(naked)]
pub unsafe extern "C" fn enter_userspace_resume(
    ctx: *const UserRegisters,
    saved_rsp_ptr: *mut u64,
) {
    core::arch::naked_asm!(
        // Save callee-saved registers (must match enter_userspace_save layout
        // so restore_kernel_context works identically).
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Save kernel RSP to *saved_rsp_ptr (rsi = 2nd argument).
        "mov [rsi], rsp",

        // Build iretq frame from UserRegisters fields.
        // UserRegisters layout: rax(0), rbx(8), rcx(16), rdx(24), rsi(32),
        // rdi(40), rbp(48), r8(56), r9(64), r10(72), r11(80), r12(88),
        // r13(96), r14(104), r15(112), rip(120), rsp(128), rflags(136)
        "push {user_ds}",                // SS
        "push qword ptr [rdi + 128]",    // RSP from ctx
        "push qword ptr [rdi + 136]",    // RFLAGS from ctx
        "push {user_cs}",                // CS
        "push qword ptr [rdi + 120]",    // RIP from ctx

        // Restore all GPRs from ctx (rdi last since it holds the pointer).
        "mov rax, [rdi + 0]",
        "mov rbx, [rdi + 8]",
        "mov rcx, [rdi + 16]",
        "mov rdx, [rdi + 24]",
        "mov rsi, [rdi + 32]",
        "mov rbp, [rdi + 48]",
        "mov r8,  [rdi + 56]",
        "mov r9,  [rdi + 64]",
        "mov r10, [rdi + 72]",
        "mov r11, [rdi + 80]",
        "mov r12, [rdi + 88]",
        "mov r13, [rdi + 96]",
        "mov r14, [rdi + 104]",
        "mov r15, [rdi + 112]",
        "mov rdi, [rdi + 40]",           // rdi last (was the pointer)

        // Transition to ring 3.
        "iretq",

        user_ds = const USER_DATA_SELECTOR,
        user_cs = const USER_CODE_SELECTOR,
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
