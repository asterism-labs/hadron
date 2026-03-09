//! Non-local jumps: `setjmp` / `longjmp` for x86_64.
//!
//! Saves and restores the callee-saved registers (RBX, RBP, R12–R15),
//! the stack pointer (RSP), and the return address (RIP). The `jmp_buf`
//! type is `[u64; 8]`, matching the declaration in `setjmp.h`.

/// `setjmp` — save calling environment for non-local jump.
///
/// Saves RBX, RBP, R12–R15, RSP, and the return address into `env`.
/// Returns 0 on direct call, or the value passed to `longjmp` on return
/// via `longjmp`.
///
/// # Safety
///
/// `env` must point to a valid `jmp_buf` (8 × u64 = 64 bytes).
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn setjmp(_env: *mut u64) -> i32 {
    // rdi = env pointer (first argument per SysV ABI).
    // Save all callee-saved registers + RSP + return address.
    core::arch::naked_asm!(
        "mov [rdi],      rbx", // env[0] = RBX
        "mov [rdi + 8],  rbp", // env[1] = RBP
        "mov [rdi + 16], r12", // env[2] = R12
        "mov [rdi + 24], r13", // env[3] = R13
        "mov [rdi + 32], r14", // env[4] = R14
        "mov [rdi + 40], r15", // env[5] = R15
        "lea rax, [rsp + 8]",  // env[6] = RSP (after setjmp returns)
        "mov [rdi + 48], rax",
        "mov rax, [rsp]", // env[7] = return address
        "mov [rdi + 56], rax",
        "xor eax, eax", // return 0
        "ret",
    );
}

/// `longjmp` — restore environment saved by `setjmp`.
///
/// Restores the registers from `env` and returns to the `setjmp` call site
/// as if `setjmp` returned `val`. If `val` is 0, it is changed to 1.
///
/// # Safety
///
/// `env` must have been filled by a prior `setjmp` call whose stack frame
/// is still valid. Calling `longjmp` after the function that called `setjmp`
/// has returned is undefined behavior.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn longjmp(_env: *const u64, _val: i32) -> ! {
    // rdi = env pointer, esi = val.
    core::arch::naked_asm!(
        "mov eax, esi",  // return value = val
        "test eax, eax", // if val == 0
        "jnz 2f",
        "mov eax, 1", // val = 1 (POSIX requirement)
        "2:",
        "mov rbx, [rdi]",      // restore RBX
        "mov rbp, [rdi + 8]",  // restore RBP
        "mov r12, [rdi + 16]", // restore R12
        "mov r13, [rdi + 24]", // restore R13
        "mov r14, [rdi + 32]", // restore R14
        "mov r15, [rdi + 40]", // restore R15
        "mov rsp, [rdi + 48]", // restore RSP
        "jmp [rdi + 56]",      // jump to saved return address
    );
}

/// `sigsetjmp` — save calling environment (signal mask variant).
///
/// On Hadron, signal masks are not yet saved/restored, so this is
/// identical to `setjmp`.
///
/// # Safety
///
/// Same requirements as `setjmp`.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn sigsetjmp(_env: *mut u64, _savemask: i32) -> i32 {
    core::arch::naked_asm!("jmp setjmp");
}

/// `siglongjmp` — restore environment saved by `sigsetjmp`.
///
/// On Hadron, signal masks are not yet saved/restored, so this is
/// identical to `longjmp`.
///
/// # Safety
///
/// Same requirements as `longjmp`.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn siglongjmp(_env: *const u64, _val: i32) -> ! {
    core::arch::naked_asm!("jmp longjmp");
}

/// `_setjmp` — BSD compatibility alias for `setjmp`.
///
/// # Safety
///
/// Same requirements as `setjmp`.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn _setjmp(_env: *mut u64) -> i32 {
    core::arch::naked_asm!("jmp setjmp");
}

/// `_longjmp` — BSD compatibility alias for `longjmp`.
///
/// # Safety
///
/// Same requirements as `longjmp`.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn _longjmp(_env: *const u64, _val: i32) -> ! {
    core::arch::naked_asm!("jmp longjmp");
}
