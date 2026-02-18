//! Raw syscall wrappers using inline assembly.
//!
//! Hadron syscall ABI: `RAX` = syscall number, arguments in
//! `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9`. Return value in `RAX`.
//! `RCX` and `R11` are clobbered by `syscall`.

/// Issue a syscall with 0 arguments.
#[inline(always)]
pub fn syscall0(nr: usize) -> isize {
    let ret: isize;
    // SAFETY: Invokes the kernel syscall handler with the given number.
    // The syscall instruction is the defined userspace-to-kernel transition.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Issue a syscall with 1 argument.
#[inline(always)]
pub fn syscall1(nr: usize, a0: usize) -> isize {
    let ret: isize;
    // SAFETY: Same as syscall0, with one argument in RDI.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Issue a syscall with 2 arguments.
#[inline(always)]
pub fn syscall2(nr: usize, a0: usize, a1: usize) -> isize {
    let ret: isize;
    // SAFETY: Same as syscall0, with two arguments in RDI, RSI.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Issue a syscall with 3 arguments.
#[inline(always)]
pub fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    // SAFETY: Same as syscall0, with three arguments in RDI, RSI, RDX.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}

/// Issue a syscall with 4 arguments.
#[inline(always)]
pub fn syscall4(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    // SAFETY: Same as syscall0, with four arguments in RDI, RSI, RDX, R10.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") nr as isize => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    ret
}
