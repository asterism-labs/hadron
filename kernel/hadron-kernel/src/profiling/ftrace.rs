//! Function tracing via `mcount` instrumentation.
//!
//! When `profile_ftrace` is enabled in Kconfig, gluon passes
//! `-Zinstrument-mcount` to rustc for kernel crates. This inserts a call
//! to `mcount` at the entry of every non-inline function.
//!
//! The `mcount` implementation here records (tsc, func_addr) pairs into
//! a per-CPU ring buffer. When tracing is stopped, all buffers are drained
//! to serial in HPRF format.
//!
//! Entry-only tracing (no exit/return patching). Sufficient for call
//! frequency and hot function identification.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::format;
use crate::arch::x86_64::hw::tsc;
use crate::id::CpuId;
use crate::percpu::{CpuLocal, cpu_count, current_cpu};

/// Whether function tracing is currently active.
static FTRACE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Per-CPU ftrace buffer entry: (tsc, func_addr).
#[derive(Clone, Copy)]
#[repr(C)]
struct FtraceEntry {
    tsc: u64,
    func_addr: u64,
}

/// Per-CPU ftrace buffer size in entries.
///
/// Each entry is 16 bytes. Default 64 KB buffer = 4096 entries.
const FTRACE_BUFFER_ENTRIES: usize = (hadron_config::PROFILE_FTRACE_BUFFER_KB as usize * 1024) / 16;

/// Per-CPU ftrace ring buffer.
///
/// Lock-free SPSC design: only the owning CPU writes (via `mcount`),
/// monotonic write index. Interrupt re-entrant because `mcount` is called
/// from function prologues which may be interrupted and re-entered.
struct FtraceBuffer {
    entries: [FtraceEntry; FTRACE_BUFFER_ENTRIES],
    /// Monotonic write index (wraps around buffer size).
    write_idx: AtomicU64,
}

impl FtraceBuffer {
    const fn new() -> Self {
        Self {
            entries: [FtraceEntry {
                tsc: 0,
                func_addr: 0,
            }; FTRACE_BUFFER_ENTRIES],
            write_idx: AtomicU64::new(0),
        }
    }

    /// Record a function entry.
    ///
    /// # Safety
    ///
    /// Only called from `mcount` on the owning CPU.
    unsafe fn record(&self, tsc: u64, func_addr: u64) {
        let idx = self.write_idx.fetch_add(1, Ordering::Relaxed);
        let slot = (idx as usize) % FTRACE_BUFFER_ENTRIES;
        // SAFETY: Single-writer per CPU. The entry is 16 bytes aligned,
        // and writes to different slots don't conflict.
        let entry_ptr = &self.entries[slot] as *const FtraceEntry as *mut FtraceEntry;
        unsafe {
            (*entry_ptr).tsc = tsc;
            (*entry_ptr).func_addr = func_addr;
        }
    }

    /// Drain all entries, calling `f` for each.
    fn drain(&self, mut f: impl FnMut(u64, u64)) {
        let total = self.write_idx.load(Ordering::Relaxed);
        if total == 0 {
            return;
        }

        let count = (total as usize).min(FTRACE_BUFFER_ENTRIES);
        let start = if total as usize > FTRACE_BUFFER_ENTRIES {
            (total as usize) % FTRACE_BUFFER_ENTRIES
        } else {
            0
        };

        for i in 0..count {
            let idx = (start + i) % FTRACE_BUFFER_ENTRIES;
            let entry = &self.entries[idx];
            f(entry.tsc, entry.func_addr);
        }

        self.write_idx.store(0, Ordering::Relaxed);
    }
}

/// Cell wrapper for per-CPU ftrace buffers.
struct FtraceBufCell(UnsafeCell<FtraceBuffer>);

// SAFETY: Each CPU only accesses its own buffer.
unsafe impl Send for FtraceBufCell {}
unsafe impl Sync for FtraceBufCell {}

impl FtraceBufCell {
    const fn new() -> Self {
        Self(UnsafeCell::new(FtraceBuffer::new()))
    }
}

/// Per-CPU ftrace buffers.
static FTRACE_BUFFERS: CpuLocal<FtraceBufCell> = {
    const CELL: FtraceBufCell = FtraceBufCell::new();
    CpuLocal::new([CELL; crate::config::MAX_CPUS])
};

/// Start function tracing.
pub fn start() {
    FTRACE_ACTIVE.store(true, Ordering::Release);
    crate::kinfo!("Function tracing started");
}

/// Stop function tracing and drain all buffers to serial.
pub fn stop() {
    FTRACE_ACTIVE.store(false, Ordering::Release);
    core::sync::atomic::fence(Ordering::SeqCst);

    crate::kinfo!("Function tracing stopped, draining buffers...");

    let cpus = cpu_count();

    format::emit_header(format::FLAG_FTRACE, 0, 0, cpus);

    for cpu in 0..cpus {
        let cpu_id = CpuId::new(cpu);
        let buf_cell = FTRACE_BUFFERS.get_for(cpu_id);
        // SAFETY: Tracing is stopped, no concurrent writes.
        let buf: &FtraceBuffer = unsafe { &*buf_cell.0.get() };
        buf.drain(|tsc_val, func_addr| {
            format::emit_ftrace_record(cpu as u8, tsc_val, func_addr);
        });
    }

    format::emit_end_of_stream();
    crate::kinfo!("Ftrace data emission complete");
}

/// Returns whether function tracing is currently active.
pub fn is_active() -> bool {
    FTRACE_ACTIVE.load(Ordering::Relaxed)
}

/// The `mcount` entry point called by `-Zinstrument-mcount` instrumentation.
///
/// When active, reads the return address (function entry point) from the
/// stack, takes a TSC timestamp, and records the entry. When inactive,
/// returns immediately after a single atomic load (~2ns).
///
/// # Safety
///
/// This is a naked function called by compiler-inserted instrumentation.
/// The return address on the stack points to the instrumented function's
/// prologue (just after the `call mcount` instruction).
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn mcount() {
    core::arch::naked_asm!(
        // Fast path: check FTRACE_ACTIVE. If false, return immediately.
        "lea rax, [rip + {active}]",
        "mov al, [rax]",
        "test al, al",
        "jz 1f",

        // Save scratch registers that we'll clobber.
        "push rdi",
        "push rsi",
        "push rdx",
        "push rcx",
        "push r8",
        "push r9",
        "push r10",
        "push r11",

        // Read TSC for timestamp.
        "rdtsc",
        "shl rdx, 32",
        "or rax, rdx",
        "mov rdi, rax",         // arg0: tsc

        // The return address (≈ function entry) is at [rsp + 64] (8 pushes * 8).
        "mov rsi, [rsp + 64]",  // arg1: func_addr (return address)

        "call {record_entry}",

        // Restore scratch registers.
        "pop r11",
        "pop r10",
        "pop r9",
        "pop r8",
        "pop rcx",
        "pop rdx",
        "pop rsi",
        "pop rdi",

        "1:",
        "ret",

        active = sym FTRACE_ACTIVE,
        record_entry = sym record_ftrace_entry,
    );
}

/// Record an ftrace entry into the current CPU's buffer.
///
/// Called from the `mcount` naked function.
#[inline(never)]
fn record_ftrace_entry(tsc_val: u64, func_addr: u64) {
    let percpu = current_cpu();
    let cpu_id = percpu.get_cpu_id();
    let buf_cell = FTRACE_BUFFERS.get_for(cpu_id);
    // SAFETY: Only the owning CPU writes to this buffer.
    let buf: &FtraceBuffer = unsafe { &*buf_cell.0.get() };
    // SAFETY: Single writer per CPU.
    unsafe { buf.record(tsc_val, func_addr) };
}
