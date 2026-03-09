//! Floating-point environment (C99 `<fenv.h>`).
//!
//! Implements `fesetround`, `fegetround`, `feclearexcept`, `fetestexcept`,
//! `feraiseexcept`, `fegetexceptflag`, `fesetexceptflag`, `fegetenv`,
//! `fesetenv`, `feholdexcept`, and `feupdateenv` using x86_64 MXCSR and x87
//! control/status words directly.
//!
//! The x87 state mirrors the MXCSR on every call so programs that use either
//! path see a consistent view.

// x86_64 MXCSR bit layout:
//   bits  0–5:  exception status flags (IE, DE, ZE, OE, UE, PE)
//   bits  7–12: exception masks (IM, DM, ZM, OM, UM, PM) — 1 = masked
//   bits 13–14: rounding mode (00=nearest, 01=down, 10=up, 11=zero)
//   bit   15:   flush-to-zero
//
// The C `FE_*` exception flag constants map 1:1 to mxcsr bits 0–5.
// The C rounding modes map to mxcsr bits 13:14 shifted left by 10.

const MXCSR_EXCEPT_MASK: u32 = 0x003f; // bits 0–5
const MXCSR_ROUND_MASK: u32 = 0x6000; // bits 13–14
const MXCSR_DEFAULT: u32 = 0x1f80; // all exceptions masked, round-to-nearest

// x87 CW rounding field is at bits 10:11 (same encoding as MXCSR 13:14).
// x87 SW status flags are at bits 0–5 (same encoding as MXCSR 0–5).

#[inline]
unsafe fn read_mxcsr() -> u32 {
    let mut v: u32 = 0;
    // SAFETY: `stmxcsr` is always available on x86_64.
    unsafe {
        core::arch::asm!("stmxcsr [{0}]", in(reg) &raw mut v as *mut u32, options(nostack));
    }
    v
}

#[inline]
unsafe fn write_mxcsr(v: u32) {
    // SAFETY: Caller ensures `v` is a valid MXCSR value.
    unsafe {
        core::arch::asm!("ldmxcsr [{0}]", in(reg) &raw const v as *const u32, options(nostack));
    }
}

#[inline]
unsafe fn read_x87_cw() -> u16 {
    let mut cw: u16 = 0;
    // SAFETY: `fstcw` is always available on x86_64.
    unsafe {
        core::arch::asm!("fstcw [{0}]", in(reg) &raw mut cw as *mut u16, options(nostack));
    }
    cw
}

#[inline]
unsafe fn write_x87_cw(cw: u16) {
    // SAFETY: Caller ensures `cw` is a valid x87 control word.
    unsafe {
        core::arch::asm!("fldcw [{0}]", in(reg) &raw const cw as *const u16, options(nostack));
    }
}

#[inline]
unsafe fn read_x87_sw() -> u16 {
    let mut sw: u16 = 0;
    // SAFETY: `fnstsw` is always available on x86_64.
    unsafe {
        core::arch::asm!("fnstsw [{0}]", in(reg) &raw mut sw as *mut u16, options(nostack));
    }
    sw
}

#[inline]
unsafe fn clear_x87_exceptions() {
    // SAFETY: `fnclex` does not cause any floating-point exceptions.
    unsafe {
        core::arch::asm!("fnclex", options(nostack));
    }
}

// Sync x87 rounding mode from MXCSR. The encoding is the same; CW bits 10:11
// correspond to MXCSR bits 13:14 but are at a different position.
#[inline]
unsafe fn sync_x87_round_from_mxcsr(mxcsr: u32) {
    let round_bits = (mxcsr & MXCSR_ROUND_MASK) >> 3; // shift 13:14 → 10:11
    // SAFETY: read_x87_cw/write_x87_cw are safe on x86_64.
    let cw = unsafe { read_x87_cw() };
    let new_cw = (cw & !(0x0c00u16)) | (round_bits as u16);
    unsafe { write_x87_cw(new_cw) };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Clear the specified floating-point exception flags.
#[unsafe(no_mangle)]
pub extern "C" fn feclearexcept(excepts: i32) -> i32 {
    let mask = (excepts as u32) & MXCSR_EXCEPT_MASK;
    // SAFETY: read/write of MXCSR and x87 status are safe inline asm.
    unsafe {
        let mxcsr = read_mxcsr();
        write_mxcsr(mxcsr & !mask);
        clear_x87_exceptions();
    }
    0
}

/// Test whether any of the specified exception flags are set.
#[unsafe(no_mangle)]
pub extern "C" fn fetestexcept(excepts: i32) -> i32 {
    let mask = (excepts as u32) & MXCSR_EXCEPT_MASK;
    // SAFETY: reading MXCSR and x87 SW is safe inline asm.
    let mxcsr_flags = unsafe { read_mxcsr() } & mask;
    let x87_flags = unsafe { read_x87_sw() } as u32 & mask;
    (mxcsr_flags | x87_flags) as i32
}

/// Raise (set) the specified floating-point exception flags.
#[unsafe(no_mangle)]
pub extern "C" fn feraiseexcept(excepts: i32) -> i32 {
    let mask = (excepts as u32) & MXCSR_EXCEPT_MASK;
    // SAFETY: read/write MXCSR is safe inline asm.
    unsafe {
        let mxcsr = read_mxcsr();
        write_mxcsr(mxcsr | mask);
    }
    0
}

/// Save the current exception flags into `*flagp`.
///
/// # Safety
///
/// `flagp` must be a valid non-null pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fegetexceptflag(flagp: *mut u32, excepts: i32) -> i32 {
    let mask = (excepts as u32) & MXCSR_EXCEPT_MASK;
    // SAFETY: reading MXCSR is safe; flagp validity is caller's responsibility.
    let flags = unsafe { read_mxcsr() } & mask;
    unsafe { *flagp = flags };
    0
}

/// Restore exception flags from `*flagp` for the specified exceptions.
///
/// # Safety
///
/// `flagp` must be a valid non-null pointer previously filled by `fegetexceptflag`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fesetexceptflag(flagp: *const u32, excepts: i32) -> i32 {
    let mask = (excepts as u32) & MXCSR_EXCEPT_MASK;
    let saved = unsafe { *flagp } & mask;
    // SAFETY: read/write MXCSR is safe inline asm.
    unsafe {
        let mxcsr = read_mxcsr();
        write_mxcsr((mxcsr & !mask) | saved);
    }
    0
}

/// Get the current rounding mode.
#[unsafe(no_mangle)]
pub extern "C" fn fegetround() -> i32 {
    // SAFETY: reading MXCSR is safe inline asm.
    let mxcsr = unsafe { read_mxcsr() };
    (mxcsr & MXCSR_ROUND_MASK) as i32
}

/// Set the rounding mode.
#[unsafe(no_mangle)]
pub extern "C" fn fesetround(round: i32) -> i32 {
    let round_bits = (round as u32) & MXCSR_ROUND_MASK;
    // SAFETY: read/write MXCSR and x87 CW are safe inline asm.
    unsafe {
        let mxcsr = read_mxcsr();
        let new_mxcsr = (mxcsr & !MXCSR_ROUND_MASK) | round_bits;
        write_mxcsr(new_mxcsr);
        sync_x87_round_from_mxcsr(new_mxcsr);
    }
    0
}

/// `fenv_t` layout: [x87_cw: u32, x87_sw: u32, mxcsr: u32]
/// We use the same 12-byte layout as glibc / musl on x86_64.

/// Save the current floating-point environment.
///
/// # Safety
///
/// `envp` must be a valid non-null pointer to an `fenv_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fegetenv(envp: *mut [u32; 3]) -> i32 {
    // SAFETY: reading x87 CW/SW and MXCSR is safe; envp validity is caller's.
    unsafe {
        let cw = read_x87_cw() as u32;
        let sw = read_x87_sw() as u32;
        let mxcsr = read_mxcsr();
        *envp = [cw, sw, mxcsr];
    }
    0
}

/// Restore the floating-point environment.
///
/// # Safety
///
/// `envp` must be a valid pointer to an `fenv_t` previously saved by
/// `fegetenv` or `feholdexcept`, or `FE_DFL_ENV` (which is `(fenv_t*)-1`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fesetenv(envp: *const [u32; 3]) -> i32 {
    // FE_DFL_ENV is (fenv_t*)-1 — restore to default state.
    if envp as usize == usize::MAX {
        // SAFETY: writing a known-good default MXCSR and x87 CW is safe.
        unsafe {
            write_mxcsr(MXCSR_DEFAULT);
            write_x87_cw(0x037f); // default x87 CW: all exceptions masked, round nearest
            clear_x87_exceptions();
        }
        return 0;
    }
    // SAFETY: envp is a valid pointer (caller guarantee).
    let [cw, _sw, mxcsr] = unsafe { *envp };
    unsafe {
        write_mxcsr(mxcsr);
        write_x87_cw(cw as u16);
    }
    0
}

/// Save the current environment and clear all exception flags.
///
/// # Safety
///
/// `envp` must be a valid non-null pointer to an `fenv_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn feholdexcept(envp: *mut [u32; 3]) -> i32 {
    // SAFETY: fegetenv and feclearexcept are safe; envp validity is caller's.
    unsafe { fegetenv(envp) };
    feclearexcept(0x3f); // FE_ALL_EXCEPT
    0
}

/// Raise any exception flags that differ between the saved env and current, then restore.
///
/// # Safety
///
/// `envp` must be a valid pointer to a saved `fenv_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn feupdateenv(envp: *const [u32; 3]) -> i32 {
    // Save which exceptions are currently set before overwriting.
    // SAFETY: reading MXCSR is safe inline asm.
    let current_excepts = unsafe { read_mxcsr() } & MXCSR_EXCEPT_MASK;
    // Restore saved environment.
    // SAFETY: envp validity is caller's responsibility.
    unsafe { fesetenv(envp) };
    // Re-raise any exceptions that were set in the current environment.
    feraiseexcept(current_excepts as i32)
}
