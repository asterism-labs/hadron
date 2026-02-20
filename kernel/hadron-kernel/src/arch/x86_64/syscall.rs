//! SYSCALL/SYSRET mechanism initialization and entry stub.
//!
//! Programs the STAR, LSTAR, and SFMASK MSRs and provides the naked
//! assembly entry point that the CPU jumps to on `syscall`.

use core::cell::UnsafeCell;

use super::registers::model_specific::{EferFlags, IA32_EFER, MSR_LSTAR, MSR_SFMASK, MSR_STAR};

/// RFLAGS bits to mask on SYSCALL entry: IF (bit 9) + DF (bit 10).
const SFMASK_VALUE: u64 = 0x600;

/// User registers saved at every SYSCALL entry.
///
/// Used by blocking syscalls (e.g. `sys_task_wait`) that longjmp via
/// `restore_kernel_context` — they need to reconstruct the user state
/// for `enter_userspace_resume`. The syscall ABI only requires callee-saved
/// registers (RBX, RBP, R12-R15), RIP, and RFLAGS to be preserved; caller-saved
/// registers may be zeroed on resume.
///
/// BSP-only; Phase 12 makes this per-CPU.
#[repr(C)]
pub struct SyscallSavedRegs {
    /// User return RIP (from CPU RCX). Offset 0.
    pub user_rip: u64,
    /// User RFLAGS (from CPU R11). Offset 8.
    pub user_rflags: u64,
    /// User RBX. Offset 16.
    pub rbx: u64,
    /// User RBP. Offset 24.
    pub rbp: u64,
    /// User R12. Offset 32.
    pub r12: u64,
    /// User R13. Offset 40.
    pub r13: u64,
    /// User R14. Offset 48.
    pub r14: u64,
    /// User R15. Offset 56.
    pub r15: u64,
}

/// Wrapper to make `UnsafeCell<SyscallSavedRegs>` usable in a `static`.
///
/// # Safety
///
/// Only written by SYSCALL entry assembly with interrupts masked (SFMASK
/// clears IF). Read by `process_task` between userspace entries. No
/// concurrent access on BSP. Phase 12 makes this per-CPU.
#[repr(transparent)]
pub struct SyncSavedRegs(UnsafeCell<SyscallSavedRegs>);

// SAFETY: See `SyncSavedRegs` doc comment — no concurrent access.
unsafe impl Sync for SyncSavedRegs {}

impl SyncSavedRegs {
    const fn new() -> Self {
        Self(UnsafeCell::new(SyscallSavedRegs {
            user_rip: 0,
            user_rflags: 0,
            rbx: 0,
            rbp: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
        }))
    }

    /// Returns a raw pointer to the inner `SyscallSavedRegs`.
    pub fn get(&self) -> *const SyscallSavedRegs {
        self.0.get()
    }
}

/// Per-CPU user registers saved at every SYSCALL entry for blocking
/// syscall resume. Indexed by CPU ID. The syscall entry stub accesses
/// the correct element via `GS:[56]` (PerCpu.saved_regs_ptr).
pub static SYSCALL_SAVED_REGS: crate::percpu::CpuLocal<SyncSavedRegs> =
    crate::percpu::CpuLocal::new([const { SyncSavedRegs::new() }; crate::percpu::MAX_CPUS]);

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
    fn syscall_dispatch(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize;
}

/// SYSCALL entry point (naked function).
///
/// On `syscall`, the CPU:
/// - Saves RIP → RCX, RFLAGS → R11
/// - Loads CS/SS from STAR
/// - Does NOT switch RSP — we must do that manually
///
/// PerCpu field offsets (with self_ptr at offset 0):
/// - `GS:[8]`  = `kernel_rsp`
/// - `GS:[16]` = `user_rsp`
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
        "mov gs:[16], rsp",         // save caller RSP to percpu.user_rsp (offset 16)
        "mov rsp, gs:[8]",          // load kernel RSP from percpu.kernel_rsp (offset 8)

        // Save caller return state and callee-saved registers
        "push rcx",                 // return RIP (saved by CPU into RCX)
        "push r11",                 // return RFLAGS (saved by CPU into R11)
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",

        // Save user callee-saved registers + RIP/RFLAGS to the per-CPU
        // SYSCALL_SAVED_REGS for blocking syscall resume (TRAP_WAIT).
        // If the syscall longjmps via restore_kernel_context, these pushed
        // values on the kernel syscall stack are lost; the static preserves them.
        // Borrow R15 as scratch (original value is at [rsp]).
        // GS:[56] = PerCpu.saved_regs_ptr → per-CPU SyncSavedRegs.
        "mov r15, gs:[56]",
        "mov [r15], rcx",           // offset 0: user_rip
        "mov [r15 + 8], r11",       // offset 8: user_rflags
        "mov [r15 + 16], rbx",      // offset 16: rbx
        "mov [r15 + 24], rbp",      // offset 24: rbp
        "mov [r15 + 32], r12",      // offset 32: r12
        "mov [r15 + 40], r13",      // offset 40: r13
        "mov [r15 + 48], r14",      // offset 48: r14 (still intact)
        // Load original r15 from stack and save it.
        "mov r14, [rsp]",           // r14 = original user r15 (at stack top)
        "mov [r15 + 56], r14",      // offset 56: r15
        // Restore clobbered registers from the stack.
        "mov r14, [rsp + 8]",       // restore r14 from its stack slot
        "mov r15, [rsp]",           // restore r15 from its stack slot

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
        "mov rsp, gs:[16]",         // restore caller RSP from percpu.user_rsp (offset 16)
        "swapgs",
        "sysretq",

        // --- Kernel return path (iretq) ---
        // R10 is caller-clobbered in the syscall ABI, safe to use as temp.
        "2:",
        "mov r10, gs:[16]",         // original RSP from percpu.user_rsp (offset 16)
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
