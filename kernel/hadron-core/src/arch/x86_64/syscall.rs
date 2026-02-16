//! SYSCALL/SYSRET mechanism initialization and entry stub.
//!
//! Programs the STAR, LSTAR, and SFMASK MSRs and provides the naked
//! assembly entry point that the CPU jumps to on `syscall`.

use super::registers::model_specific::{EferFlags, IA32_EFER, MSR_LSTAR, MSR_SFMASK, MSR_STAR};

/// RFLAGS bits to mask on SYSCALL entry: IF (bit 9) + DF (bit 10).
const SFMASK_VALUE: u64 = 0x600;

/// Initializes the SYSCALL/SYSRET mechanism.
///
/// Programs the following MSRs:
/// - **EFER**: Sets `SCE` (System Call Enable) bit
/// - **STAR**: `STAR[47:32]` = kernel CS (0x08), `STAR[63:48]` = sysret base (0x10)
///   - SYSCALL loads: CS = 0x08, SS = 0x08+8 = 0x10
///   - SYSRET loads:  SS = 0x10+8 = 0x18 (user data), CS = 0x10+16 = 0x20 (user code)
///     (both OR'd with RPL=3)
/// - **LSTAR**: Address of [`syscall_entry`]
/// - **SFMASK**: Masks IF and DF in RFLAGS on entry
///
/// # Safety
///
/// - Must be called after GDT init (segments must be in the expected order).
/// - Must be called after `percpu::init_gs_base()`.
/// - Must be called exactly once per CPU.
pub unsafe fn init() {
    unsafe {
        // Enable SYSCALL/SYSRET in EFER
        let efer = IA32_EFER.read();
        IA32_EFER.write(efer | EferFlags::SYSTEM_CALL_ENABLE.bits());

        // STAR: kernel CS in bits 32-47, sysret base in bits 48-63
        let star = (0x08u64 << 32) | (0x10u64 << 48);
        MSR_STAR.write(star);

        // LSTAR: syscall entry point
        MSR_LSTAR.write(syscall_entry as *const () as usize as u64);

        // SFMASK: mask IF and DF on entry
        MSR_SFMASK.write(SFMASK_VALUE);
    }

    crate::kdebug!("SYSCALL/SYSRET initialized");
}

// Declared in hadron-kernel, linked via `extern "C"`.
unsafe extern "C" {
    fn syscall_dispatch(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize)
        -> isize;
}

/// SYSCALL entry point (naked function).
///
/// On `syscall`, the CPU:
/// - Saves RIP → RCX, RFLAGS → R11
/// - Loads CS/SS from STAR
/// - Does NOT switch RSP — we must do that manually
///
/// Linux syscall convention:
/// - Syscall number in RAX
/// - Args: RDI, RSI, RDX, R10, R8, R9
/// - R10 replaces RCX (since CPU clobbers RCX with return RIP)
/// - Return value in RAX
///
/// Remapped to SysV C convention for `syscall_dispatch(nr, a0, a1, a2, a3, a4)`:
/// - RDI = nr (from RAX)
/// - RSI = a0 (from RDI)
/// - RDX = a1 (from RSI)
/// - RCX = a2 (from RDX)
/// - R8  = a3 (from R10)
/// - R9  = a4 (from R8)
///
/// The exit path checks if the return RIP is in kernel space (bit 63 set)
/// and uses `iretq` instead of `sysretq` for ring 0 callers, since `sysretq`
/// unconditionally loads ring 3 CS/SS.
#[unsafe(naked)]
unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        // Switch to kernel GS and stack
        "swapgs",
        "mov gs:[8], rsp",          // save caller RSP to percpu.user_rsp
        "mov rsp, gs:[0]",          // load kernel RSP from percpu.kernel_rsp

        // Save caller return state and callee-saved registers
        "push rcx",                 // return RIP (saved by CPU into RCX)
        "push r11",                 // return RFLAGS (saved by CPU into R11)
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Remap syscall registers to SysV C calling convention.
        // Incoming:  RAX=nr, RDI=a0, RSI=a1, RDX=a2, R10=a3, R8=a4, R9=a5
        // Outgoing:  RDI=nr, RSI=a0, RDX=a1, RCX=a2, R8=a3,  R9=a4
        // Chain dependency: must save before overwriting.
        "mov rcx, rdx",             // C arg3 = a2 (save RDX before overwrite)
        "mov rdx, rsi",             // C arg2 = a1 (save RSI before overwrite)
        "mov rsi, rdi",             // C arg1 = a0
        "mov rdi, rax",             // C arg0 = syscall number
        "mov r9, r8",               // C arg5 = a4 (save R8 before overwrite)
        "mov r8, r10",              // C arg4 = a3

        // Call the Rust dispatch function (return value in RAX)
        "call {dispatch}",

        // Restore callee-saved registers and caller return state
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "pop r11",                   // return RFLAGS
        "pop rcx",                   // return RIP

        // Check if returning to kernel (bit 63 set) or user space.
        // sysretq unconditionally loads ring 3 CS/SS, so ring 0 callers
        // must use iretq instead.
        "test rcx, rcx",
        "js 2f",

        // --- User return path (sysretq) ---
        "mov rsp, gs:[8]",          // restore caller RSP
        "swapgs",
        "sysretq",

        // --- Kernel return path (iretq) ---
        // R10 is caller-clobbered in the syscall ABI, safe to use as temp.
        "2:",
        "mov r10, gs:[8]",          // original RSP
        "swapgs",
        // Build iretq frame: SS, RSP, RFLAGS, CS, RIP
        "push 0x10",                // SS = kernel data selector
        "push r10",                 // RSP = original stack pointer
        "push r11",                 // RFLAGS
        "push 0x08",                // CS = kernel code selector
        "push rcx",                 // RIP = return address
        "iretq",

        dispatch = sym syscall_dispatch,
    );
}
