//! Exception and interrupt entry: the assembly stubs, the saved register
//! frame and the Rust dispatcher.
//!
//! Each of the 256 vectors has a stub. Stubs for vectors without a CPU
//! error code push a zero in its place, then every stub pushes its vector
//! and jumps to a common entry. Vectors 0 to 31 are exceptions. Vectors 32
//! to 255 go to [`irq::dispatch`]. The entry saves the general registers,
//! calls [`trap_dispatch`] with a pointer to the [`TrapFrame`] on the
//! stack, restores the registers and returns with `iretq`.
//!
//! The CPU aligns RSP to 16 bytes before pushing its frame (SDM Vol. 3A
//! §7.14.2). Its five words, the error code, the vector and the fifteen
//! saved registers make 22 words, so RSP is still 16-byte aligned at the
//! `call`, as the System V ABI wants.
//!
//! Nothing runs in ring 3 yet, so the entry does not `swapgs`.

use core::arch::global_asm;
use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use arch_x86_64::Exception;

use super::{cpu, irq};
use crate::console::kprintln;

/// Number of vectors with a stub: all of them.
pub(crate) const STUBS: usize = 256;

// The vectors in the two `.irp` lists must match
// `Exception::has_error_code`, which the arch crate's tests pin to SDM
// Vol. 3A Table 7-1. Vectors 29 and 30 are AMD's #VC and #SX.
global_asm!(
    ".pushsection .text.hadron_trap, \"ax\"",
    ".irp vector, 0,1,2,3,4,5,6,7,9,15,16,18,19,20,22,23,24,25,26,27,28,31",
    "hadron_trap_stub_\\vector:",
    "    push 0",
    "    push \\vector",
    "    jmp hadron_trap_entry",
    ".endr",
    ".irp vector, 8,10,11,12,13,14,17,21,29,30",
    "hadron_trap_stub_\\vector:",
    "    push \\vector",
    "    jmp hadron_trap_entry",
    ".endr",
    // Interrupt stubs, 16 bytes apart, so the table below can compute
    // their addresses. No interrupt has an error code.
    ".balign 16",
    "hadron_irq_stubs:",
    ".set hadron_vector, 32",
    ".rept 224",
    "    .balign 16",
    "    push 0",
    "    push hadron_vector",
    "    jmp hadron_trap_entry",
    "    .set hadron_vector, hadron_vector + 1",
    ".endr",
    "",
    "hadron_trap_entry:",
    "    push r15",
    "    push r14",
    "    push r13",
    "    push r12",
    "    push r11",
    "    push r10",
    "    push r9",
    "    push r8",
    "    push rbp",
    "    push rdi",
    "    push rsi",
    "    push rdx",
    "    push rcx",
    "    push rbx",
    "    push rax",
    // The interrupted code may have set DF. The ABI wants it clear.
    "    cld",
    "    mov rdi, rsp",
    "    call {dispatch}",
    "    pop rax",
    "    pop rbx",
    "    pop rcx",
    "    pop rdx",
    "    pop rsi",
    "    pop rdi",
    "    pop rbp",
    "    pop r8",
    "    pop r9",
    "    pop r10",
    "    pop r11",
    "    pop r12",
    "    pop r13",
    "    pop r14",
    "    pop r15",
    // Drop the vector and the error code.
    "    add rsp, 16",
    "    iretq",
    ".popsection",
    "",
    ".pushsection .rodata.hadron_trap, \"a\"",
    ".balign 8",
    ".global hadron_trap_stubs",
    "hadron_trap_stubs:",
    ".irp vector, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31",
    "    .quad hadron_trap_stub_\\vector",
    ".endr",
    ".set hadron_vector, 0",
    ".rept 224",
    "    .quad hadron_irq_stubs + hadron_vector * 16",
    "    .set hadron_vector, hadron_vector + 1",
    ".endr",
    ".popsection",
    dispatch = sym trap_dispatch,
);

unsafe extern "C" {
    /// Addresses of the stubs, indexed by vector.
    #[link_name = "hadron_trap_stubs"]
    static TRAP_STUBS: [u64; STUBS];
}

/// Addresses of the stubs, indexed by vector.
pub(crate) fn stubs() -> &'static [u64; STUBS] {
    // SAFETY: the assembly above defines the table in read-only data with
    // one address per vector.
    unsafe { &TRAP_STUBS }
}

/// The registers saved on trap entry, lowest address first. The
/// layout must match the pushes in the entry stub.
#[derive(Debug)]
#[repr(C)]
pub(crate) struct TrapFrame {
    pub(crate) rax: u64,
    pub(crate) rbx: u64,
    pub(crate) rcx: u64,
    pub(crate) rdx: u64,
    pub(crate) rsi: u64,
    pub(crate) rdi: u64,
    pub(crate) rbp: u64,
    pub(crate) r8: u64,
    pub(crate) r9: u64,
    pub(crate) r10: u64,
    pub(crate) r11: u64,
    pub(crate) r12: u64,
    pub(crate) r13: u64,
    pub(crate) r14: u64,
    pub(crate) r15: u64,
    /// Pushed by the stub.
    pub(crate) vector: u64,
    /// Pushed by the CPU, or zero from the stub.
    pub(crate) error_code: u64,
    // Pushed by the CPU.
    pub(crate) rip: u64,
    pub(crate) cs: u64,
    pub(crate) rflags: u64,
    pub(crate) rsp: u64,
    pub(crate) ss: u64,
}

const _: () = assert!(size_of::<TrapFrame>() == 22 * 8);

impl fmt::Display for TrapFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rows = [
            [
                ("rip", self.rip),
                ("rsp", self.rsp),
                ("rflags", self.rflags),
            ],
            [("rax", self.rax), ("rbx", self.rbx), ("rcx", self.rcx)],
            [("rdx", self.rdx), ("rsi", self.rsi), ("rdi", self.rdi)],
            [("rbp", self.rbp), ("r8", self.r8), ("r9", self.r9)],
            [("r10", self.r10), ("r11", self.r11), ("r12", self.r12)],
            [("r13", self.r13), ("r14", self.r14), ("r15", self.r15)],
        ];
        for row in rows {
            for (name, value) in row {
                write!(f, "  {name:>6} {value:#018x}")?;
            }
            writeln!(f)?;
        }
        write!(
            f,
            "  {:>6} {:#06x}  {:>6} {:#06x}",
            "cs", self.cs, "ss", self.ss
        )
    }
}

/// Number of breakpoints handled, for the kernel tests.
static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);

/// Number of breakpoint exceptions handled so far.
pub(crate) fn breakpoints() -> u64 {
    BREAKPOINTS.load(Ordering::Relaxed)
}

/// Called by the entry stub for every vector.
extern "C" fn trap_dispatch(frame: &mut TrapFrame) {
    if let Ok(vector @ irq::FIRST..) = u8::try_from(frame.vector) {
        irq::dispatch(vector);
        return;
    }
    match Exception::from_vector(frame.vector) {
        Some(Exception::Breakpoint) => {
            BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
        }
        exception => fatal(frame, exception),
    }
}

/// Prints the frame and panics, for every exception the kernel cannot
/// recover from yet.
fn fatal(frame: &TrapFrame, exception: Option<Exception>) -> ! {
    let name = exception.map_or("reserved exception", Exception::name);
    kprintln!(
        "hadron: {name}, vector {}, error code {:#x}",
        frame.vector,
        frame.error_code
    );
    if exception == Some(Exception::PageFault) {
        kprintln!("  address {:#018x}", cpu::cr2());
    }
    kprintln!("{frame}");
    panic!("{name} at {:#x}", frame.rip);
}
