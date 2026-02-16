//! Kernel panic backtrace support.
//!
//! Provides symbolic backtraces on panic by walking the RBP frame pointer chain
//! and resolving addresses against an HBTF (Hadron Backtrace Format) table loaded
//! at boot time as a Limine module.
//!
//! # HBTF Format
//!
//! The HBTF binary contains sorted symbol and line tables for binary search lookup.
//! See `xtask/src/hbtf.rs` for the format specification.

use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::sync::SpinLock;

/// Maximum number of frames to capture in a backtrace.
const MAX_FRAMES: usize = 32;

/// HBTF file magic bytes.
const HBTF_MAGIC: [u8; 4] = *b"HBTF";

/// HBTF format version we support.
const HBTF_VERSION: u32 = 1;

/// Size of the HBTF file header.
const HBTF_HEADER_SIZE: usize = 32;

/// Size of a symbol entry in the HBTF binary.
const SYM_ENTRY_SIZE: usize = 20;

/// Size of a line entry in the HBTF binary.
const LINE_ENTRY_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// The raw HBTF data slice, set once at boot.
static HBTF_DATA: SpinLock<Option<&'static [u8]>> = SpinLock::new(None);

/// Kernel virtual base address for offset-to-address conversion.
static KERNEL_VIRT_BASE: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the backtrace system with HBTF data and the kernel virtual base.
///
/// Must be called once during boot, before any panic could benefit from backtraces.
/// After this call, the global state is read-only.
pub fn init(hbtf_data: &'static [u8], kernel_virt_base: u64) {
    // Validate the HBTF header
    if hbtf_data.len() < HBTF_HEADER_SIZE {
        hadron_core::kwarn!("HBTF: data too short ({} bytes), backtraces disabled", hbtf_data.len());
        return;
    }

    if hbtf_data[..4] != HBTF_MAGIC {
        hadron_core::kwarn!("HBTF: invalid magic, backtraces disabled");
        return;
    }

    let version = u32::from_le_bytes([hbtf_data[4], hbtf_data[5], hbtf_data[6], hbtf_data[7]]);
    if version != HBTF_VERSION {
        hadron_core::kwarn!("HBTF: unsupported version {version}, backtraces disabled");
        return;
    }

    let sym_count = u32::from_le_bytes([hbtf_data[8], hbtf_data[9], hbtf_data[10], hbtf_data[11]]);
    let line_count =
        u32::from_le_bytes([hbtf_data[16], hbtf_data[17], hbtf_data[18], hbtf_data[19]]);

    KERNEL_VIRT_BASE.store(kernel_virt_base, Ordering::Relaxed);
    *HBTF_DATA.lock() = Some(hbtf_data);

    hadron_core::kinfo!(
        "Backtrace: loaded HBTF ({} symbols, {} lines)",
        sym_count,
        line_count
    );
}

/// Print a backtrace to the given writer. Safe to call from panic context.
///
/// If HBTF data is not available, prints raw hex addresses.
/// If addresses can't be resolved, prints them without symbol info.
pub fn panic_backtrace(writer: &mut impl Write) {
    let frames = capture_backtrace();
    let frame_count = frames.len();

    if frame_count == 0 {
        let _ = write!(writer, "Backtrace: no frames captured\n");
        return;
    }

    let _ = write!(writer, "Backtrace ({frame_count} frames):\n");

    // Try to lock HBTF data — if we can't (e.g., panic while holding the lock),
    // fall back to raw addresses.
    let hbtf_guard = HBTF_DATA.try_lock();
    let hbtf_data = hbtf_guard.as_ref().and_then(|g| g.as_ref().copied());

    let kernel_base = KERNEL_VIRT_BASE.load(Ordering::Relaxed);

    for (i, &addr) in frames.iter().enumerate() {
        if let Some(hbtf) = hbtf_data {
            print_frame_symbolicated(writer, i, addr, hbtf, kernel_base);
        } else {
            let _ = write!(writer, "  #{i}: {addr:#018x}\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Frame pointer stack walker
// ---------------------------------------------------------------------------

/// Captured return addresses from the call stack.
struct FrameBuffer {
    addrs: [u64; MAX_FRAMES],
    len: usize,
}

impl FrameBuffer {
    fn len(&self) -> usize {
        self.len
    }

    fn iter(&self) -> core::slice::Iter<'_, u64> {
        self.addrs[..self.len].iter()
    }
}

/// Walk the RBP frame pointer chain and capture return addresses.
///
/// Each frame on an x86_64 stack looks like:
/// ```text
///   [rbp+8] = return address
///   [rbp]   = saved caller's rbp
/// ```
fn capture_backtrace() -> FrameBuffer {
    let mut buf = FrameBuffer {
        addrs: [0; MAX_FRAMES],
        len: 0,
    };

    let mut rbp: u64;
    // SAFETY: Reading the current RBP register is always safe.
    unsafe {
        core::arch::asm!("mov {}, rbp", out(reg) rbp, options(nomem, nostack, preserves_flags));
    }

    let mut prev_rbp = 0u64;

    while buf.len < MAX_FRAMES {
        // Terminate on null or non-canonical RBP
        if rbp == 0 {
            break;
        }

        // Must be in kernel space (high half) and aligned to 8 bytes
        if rbp < 0xFFFF_8000_0000_0000 || rbp & 0x7 != 0 {
            break;
        }

        // Detect loops (RBP should always move toward higher addresses in a normal stack)
        if rbp == prev_rbp {
            break;
        }

        // Read return address and next RBP
        // SAFETY: We validated that RBP is a kernel-space, aligned address.
        // The frame pointer chain was set up by the compiler with
        // -C force-frame-pointers=yes. Each frame has [rbp]=saved_rbp
        // and [rbp+8]=return_address.
        let (next_rbp, ret_addr) = unsafe {
            let rbp_ptr = rbp as *const u64;
            (*rbp_ptr, *rbp_ptr.add(1))
        };

        // Only record kernel-space return addresses
        if ret_addr >= 0xFFFF_8000_0000_0000 {
            buf.addrs[buf.len] = ret_addr;
            buf.len += 1;
        }

        prev_rbp = rbp;
        rbp = next_rbp;
    }

    buf
}

// ---------------------------------------------------------------------------
// HBTF lookup and formatting
// ---------------------------------------------------------------------------

/// Print a single frame with symbol and line info from HBTF data.
fn print_frame_symbolicated(
    writer: &mut impl Write,
    index: usize,
    addr: u64,
    hbtf: &[u8],
    kernel_base: u64,
) {
    let offset = addr.wrapping_sub(kernel_base);

    // Look up symbol
    let sym = lookup_symbol(hbtf, offset);

    // Look up line info
    let line = lookup_line(hbtf, offset);

    let _ = write!(writer, "  #{index}: {addr:#018x}");

    if let Some((name, func_offset)) = sym {
        let _ = write!(writer, " - {name}+{func_offset:#x}");
    }

    if let Some((file, line_num)) = line {
        let _ = write!(writer, " ({file}:{line_num})");
    }

    let _ = write!(writer, "\n");
}

/// Binary search the HBTF symbol table for a function containing `offset`.
///
/// Returns `(name, offset_within_function)` if found.
fn lookup_symbol<'a>(hbtf: &'a [u8], offset: u64) -> Option<(&'a str, u64)> {
    let sym_count =
        u32::from_le_bytes([hbtf[8], hbtf[9], hbtf[10], hbtf[11]]) as usize;
    let sym_offset =
        u32::from_le_bytes([hbtf[12], hbtf[13], hbtf[14], hbtf[15]]) as usize;
    let strings_offset =
        u32::from_le_bytes([hbtf[24], hbtf[25], hbtf[26], hbtf[27]]) as usize;

    if sym_count == 0 {
        return None;
    }

    // Binary search: find the last symbol with addr <= offset
    let mut lo = 0usize;
    let mut hi = sym_count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let entry_off = sym_offset + mid * SYM_ENTRY_SIZE;
        if entry_off + SYM_ENTRY_SIZE > hbtf.len() {
            return None;
        }
        let sym_addr = u64::from_le_bytes([
            hbtf[entry_off],
            hbtf[entry_off + 1],
            hbtf[entry_off + 2],
            hbtf[entry_off + 3],
            hbtf[entry_off + 4],
            hbtf[entry_off + 5],
            hbtf[entry_off + 6],
            hbtf[entry_off + 7],
        ]);
        if sym_addr <= offset {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    if lo == 0 {
        return None;
    }

    // The candidate is at lo - 1
    let idx = lo - 1;
    let entry_off = sym_offset + idx * SYM_ENTRY_SIZE;
    if entry_off + SYM_ENTRY_SIZE > hbtf.len() {
        return None;
    }

    let sym_addr = u64::from_le_bytes([
        hbtf[entry_off],
        hbtf[entry_off + 1],
        hbtf[entry_off + 2],
        hbtf[entry_off + 3],
        hbtf[entry_off + 4],
        hbtf[entry_off + 5],
        hbtf[entry_off + 6],
        hbtf[entry_off + 7],
    ]);
    let sym_size = u32::from_le_bytes([
        hbtf[entry_off + 8],
        hbtf[entry_off + 9],
        hbtf[entry_off + 10],
        hbtf[entry_off + 11],
    ]);
    let name_off = u32::from_le_bytes([
        hbtf[entry_off + 12],
        hbtf[entry_off + 13],
        hbtf[entry_off + 14],
        hbtf[entry_off + 15],
    ]) as usize;

    // Check if offset falls within the symbol's range
    let func_offset = offset - sym_addr;
    if sym_size > 0 && func_offset >= u64::from(sym_size) {
        return None;
    }

    // Read NUL-terminated name from string pool
    let name_start = strings_offset + name_off;
    let name = read_nul_str(hbtf, name_start)?;

    Some((name, func_offset))
}

/// Binary search the HBTF line table for the entry at or before `offset`.
///
/// Returns `(file, line)` if found.
fn lookup_line<'a>(hbtf: &'a [u8], offset: u64) -> Option<(&'a str, u32)> {
    let line_count =
        u32::from_le_bytes([hbtf[16], hbtf[17], hbtf[18], hbtf[19]]) as usize;
    let line_offset =
        u32::from_le_bytes([hbtf[20], hbtf[21], hbtf[22], hbtf[23]]) as usize;
    let strings_offset =
        u32::from_le_bytes([hbtf[24], hbtf[25], hbtf[26], hbtf[27]]) as usize;

    if line_count == 0 {
        return None;
    }

    // Binary search: find the last line entry with addr <= offset
    let mut lo = 0usize;
    let mut hi = line_count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let entry_off = line_offset + mid * LINE_ENTRY_SIZE;
        if entry_off + LINE_ENTRY_SIZE > hbtf.len() {
            return None;
        }
        let line_addr = u64::from_le_bytes([
            hbtf[entry_off],
            hbtf[entry_off + 1],
            hbtf[entry_off + 2],
            hbtf[entry_off + 3],
            hbtf[entry_off + 4],
            hbtf[entry_off + 5],
            hbtf[entry_off + 6],
            hbtf[entry_off + 7],
        ]);
        if line_addr <= offset {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    if lo == 0 {
        return None;
    }

    let idx = lo - 1;
    let entry_off = line_offset + idx * LINE_ENTRY_SIZE;
    if entry_off + LINE_ENTRY_SIZE > hbtf.len() {
        return None;
    }

    let file_off = u32::from_le_bytes([
        hbtf[entry_off + 8],
        hbtf[entry_off + 9],
        hbtf[entry_off + 10],
        hbtf[entry_off + 11],
    ]) as usize;
    let line_num = u32::from_le_bytes([
        hbtf[entry_off + 12],
        hbtf[entry_off + 13],
        hbtf[entry_off + 14],
        hbtf[entry_off + 15],
    ]);

    let file_start = strings_offset + file_off;
    let file = read_nul_str(hbtf, file_start)?;

    Some((file, line_num))
}

/// Read a NUL-terminated string from the HBTF data at the given offset.
fn read_nul_str(data: &[u8], offset: usize) -> Option<&str> {
    if offset >= data.len() {
        return None;
    }
    let remaining = &data[offset..];
    let nul_pos = remaining.iter().position(|&b| b == 0)?;
    core::str::from_utf8(&remaining[..nul_pos]).ok()
}
