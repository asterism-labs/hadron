//! Kernel panic backtrace support.
//!
//! Provides symbolic backtraces on panic by walking the RBP frame pointer chain
//! and resolving addresses against HKIF (Hadron Kernel Image Format) data
//! embedded in the kernel binary at link time.
//!
//! The HKIF blob is placed in the `.hadron_hkif` linker section by the two-pass
//! build. It contains a section directory pointing to sorted symbol and line
//! tables for binary search lookup.

use core::fmt::Write;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::sync::SpinLock;

/// Maximum number of frames to capture in a backtrace.
const MAX_FRAMES: usize = 32;

/// HKIF magic bytes.
const HKIF_MAGIC: [u8; 4] = *b"HKIF";

/// HKIF format version we support.
const HKIF_VERSION: u16 = 1;

/// HKIF header size in bytes.
const HKIF_HEADER_SIZE: usize = 64;

/// HKIF section directory entry size in bytes.
const DIR_ENTRY_SIZE: usize = 16;

/// Section type: symbols (20 bytes per entry).
const SECTION_SYMBOLS: u32 = 2;

/// Section type: lines (16 bytes per entry).
const SECTION_LINES: u32 = 3;

/// Section type: string pool.
const SECTION_STRINGS: u32 = 4;

/// Size of a symbol entry.
const SYM_ENTRY_SIZE: usize = 20;

/// Size of a line entry.
const LINE_ENTRY_SIZE: usize = 16;

// ---------------------------------------------------------------------------
// Parsed HKIF section locations
// ---------------------------------------------------------------------------

/// Offsets and sizes of backtrace-relevant sections within the HKIF blob.
struct HkifSections {
    /// Offset of symbol table from HKIF start.
    sym_offset: usize,
    /// Number of symbol entries (sym_size / SYM_ENTRY_SIZE).
    sym_count: usize,
    /// Offset of line table from HKIF start.
    line_offset: usize,
    /// Number of line entries (line_size / LINE_ENTRY_SIZE).
    line_count: usize,
    /// Offset of string pool from HKIF start.
    strings_offset: usize,
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// The raw HKIF data slice and parsed section info, set once at boot.
static HKIF_STATE: SpinLock<Option<HkifState>> = SpinLock::leveled("HKIF_STATE", 4, None);

/// Kernel virtual base address for offset-to-address conversion.
static KERNEL_VIRT_BASE: AtomicU64 = AtomicU64::new(0);

/// Combined state for the HKIF backtrace system.
struct HkifState {
    data: &'static [u8],
    sections: HkifSections,
}

// ---------------------------------------------------------------------------
// Linker section accessor
// ---------------------------------------------------------------------------

hadron_linkset::declare_linkset_blob! {
    /// Returns the embedded HKIF data from the `.hadron_hkif` linker section.
    fn hkif_data() -> &[u8],
    section = "hadron_hkif"
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize the backtrace system from the embedded HKIF section.
///
/// Reads the `.hadron_hkif` linker section, validates the HKIF header,
/// and parses the section directory to locate symbol/line/string data.
///
/// Must be called once during boot. The `kernel_virt_base` is the lowest
/// PT_LOAD virtual address of the kernel image.
pub fn init_from_embedded(kernel_virt_base: u64) {
    let data = hkif_data();

    if data.is_empty() {
        crate::kwarn!("HKIF: empty .hadron_hkif section, backtraces disabled");
        return;
    }

    if let Some(state) = parse_hkif(data) {
        let sym_count = state.sections.sym_count;
        let line_count = state.sections.line_count;

        KERNEL_VIRT_BASE.store(kernel_virt_base, Ordering::Relaxed);
        // lock_unchecked: runs during single-threaded early boot (step 2b)
        // before interrupts are enabled (step 11). No contention possible.
        *HKIF_STATE.lock_unchecked() = Some(state);

        crate::kinfo!(
            "Backtrace: loaded HKIF ({} symbols, {} lines, {} bytes)",
            sym_count,
            line_count,
            data.len(),
        );
    }
}

/// Returns `true` if HKIF symbol data has been loaded and symbolication is
/// available. Test binaries that skip the two-pass link will return `false`.
pub fn is_available() -> bool {
    HKIF_STATE
        .try_lock()
        .is_some_and(|guard| guard.is_some())
}

/// Print a backtrace to the given writer. Safe to call from panic context.
///
/// If HKIF data is not available, prints raw hex addresses.
pub fn panic_backtrace(writer: &mut impl Write) {
    let frames = capture_backtrace();
    let frame_count = frames.len();

    if frame_count == 0 {
        let _ = write!(writer, "Backtrace: no frames captured\n");
        return;
    }

    let _ = write!(writer, "Backtrace ({frame_count} frames):\n");

    // Try to lock HKIF state — if we can't (e.g., panic while holding the lock),
    // fall back to raw addresses.
    let guard = HKIF_STATE.try_lock();
    let kernel_base = KERNEL_VIRT_BASE.load(Ordering::Relaxed);

    for (i, &addr) in frames.iter().enumerate() {
        if let Some(Some(state)) = guard.as_deref() {
            print_frame_symbolicated(writer, i, addr, state, kernel_base);
        } else {
            let _ = write!(writer, "  #{i}: {addr:#018x}\n");
        }
    }
}

// ---------------------------------------------------------------------------
// HKIF parsing
// ---------------------------------------------------------------------------

/// Parse and validate the HKIF header and section directory.
fn parse_hkif(data: &'static [u8]) -> Option<HkifState> {
    if data.len() < HKIF_HEADER_SIZE {
        crate::kwarn!(
            "HKIF: data too short ({} bytes), backtraces disabled",
            data.len()
        );
        return None;
    }

    if data[..4] != HKIF_MAGIC {
        crate::kwarn!("HKIF: invalid magic, backtraces disabled");
        return None;
    }

    let version = u16::from_le_bytes([data[4], data[5]]);
    if version != HKIF_VERSION {
        crate::kwarn!("HKIF: unsupported version {version}, backtraces disabled");
        return None;
    }

    let section_count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let dir_offset = u32::from_le_bytes([data[12], data[13], data[14], data[15]]) as usize;

    // Parse section directory.
    let mut sym_offset = 0usize;
    let mut sym_size = 0usize;
    let mut line_offset = 0usize;
    let mut line_size = 0usize;
    let mut strings_offset = 0usize;

    for i in 0..section_count {
        let entry_start = dir_offset + i * DIR_ENTRY_SIZE;
        if entry_start + DIR_ENTRY_SIZE > data.len() {
            break;
        }

        let sec_type = u32::from_le_bytes([
            data[entry_start],
            data[entry_start + 1],
            data[entry_start + 2],
            data[entry_start + 3],
        ]);
        let sec_offset = u32::from_le_bytes([
            data[entry_start + 4],
            data[entry_start + 5],
            data[entry_start + 6],
            data[entry_start + 7],
        ]) as usize;
        let sec_size = u32::from_le_bytes([
            data[entry_start + 8],
            data[entry_start + 9],
            data[entry_start + 10],
            data[entry_start + 11],
        ]) as usize;

        match sec_type {
            SECTION_SYMBOLS => {
                sym_offset = sec_offset;
                sym_size = sec_size;
            }
            SECTION_LINES => {
                line_offset = sec_offset;
                line_size = sec_size;
            }
            SECTION_STRINGS => {
                strings_offset = sec_offset;
            }
            _ => {} // Unknown section types are skipped (forward compat).
        }
    }

    Some(HkifState {
        data,
        sections: HkifSections {
            sym_offset,
            sym_count: sym_size / SYM_ENTRY_SIZE,
            line_offset,
            line_count: line_size / LINE_ENTRY_SIZE,
            strings_offset,
        },
    })
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
// HKIF lookup and formatting
// ---------------------------------------------------------------------------

/// Print a single frame with symbol and line info from HKIF data.
fn print_frame_symbolicated(
    writer: &mut impl Write,
    index: usize,
    addr: u64,
    state: &HkifState,
    kernel_base: u64,
) {
    let offset = addr.wrapping_sub(kernel_base);

    let sym = lookup_symbol(state, offset);
    let line = lookup_line(state, offset);

    let _ = write!(writer, "  #{index}: {addr:#018x}");

    if let Some((name, func_offset)) = sym {
        let _ = write!(writer, " - {name}+{func_offset:#x}");
    }

    if let Some((file, line_num)) = line {
        let _ = write!(writer, " ({file}:{line_num})");
    }

    let _ = write!(writer, "\n");
}

/// Binary search the symbol table for a function containing `offset`.
///
/// Returns `(name, offset_within_function)` if found.
fn lookup_symbol<'a>(state: &'a HkifState, offset: u64) -> Option<(&'a str, u64)> {
    let s = &state.sections;
    let data = state.data;

    if s.sym_count == 0 {
        return None;
    }

    // Binary search: find the last symbol with addr <= offset.
    let mut lo = 0usize;
    let mut hi = s.sym_count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let entry_off = s.sym_offset + mid * SYM_ENTRY_SIZE;
        if entry_off + SYM_ENTRY_SIZE > data.len() {
            return None;
        }
        let sym_addr = u64::from_le_bytes([
            data[entry_off],
            data[entry_off + 1],
            data[entry_off + 2],
            data[entry_off + 3],
            data[entry_off + 4],
            data[entry_off + 5],
            data[entry_off + 6],
            data[entry_off + 7],
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

    let idx = lo - 1;
    let entry_off = s.sym_offset + idx * SYM_ENTRY_SIZE;
    if entry_off + SYM_ENTRY_SIZE > data.len() {
        return None;
    }

    let sym_addr = u64::from_le_bytes([
        data[entry_off],
        data[entry_off + 1],
        data[entry_off + 2],
        data[entry_off + 3],
        data[entry_off + 4],
        data[entry_off + 5],
        data[entry_off + 6],
        data[entry_off + 7],
    ]);
    let sym_size = u32::from_le_bytes([
        data[entry_off + 8],
        data[entry_off + 9],
        data[entry_off + 10],
        data[entry_off + 11],
    ]);
    let name_off = u32::from_le_bytes([
        data[entry_off + 12],
        data[entry_off + 13],
        data[entry_off + 14],
        data[entry_off + 15],
    ]) as usize;

    let func_offset = offset - sym_addr;
    if sym_size > 0 && func_offset >= u64::from(sym_size) {
        return None;
    }

    let name_start = s.strings_offset + name_off;
    let name = read_nul_str(data, name_start)?;

    Some((name, func_offset))
}

/// Binary search the line table for the entry at or before `offset`.
///
/// Returns `(file, line)` if found.
fn lookup_line<'a>(state: &'a HkifState, offset: u64) -> Option<(&'a str, u32)> {
    let s = &state.sections;
    let data = state.data;

    if s.line_count == 0 {
        return None;
    }

    let mut lo = 0usize;
    let mut hi = s.line_count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let entry_off = s.line_offset + mid * LINE_ENTRY_SIZE;
        if entry_off + LINE_ENTRY_SIZE > data.len() {
            return None;
        }
        let line_addr = u64::from_le_bytes([
            data[entry_off],
            data[entry_off + 1],
            data[entry_off + 2],
            data[entry_off + 3],
            data[entry_off + 4],
            data[entry_off + 5],
            data[entry_off + 6],
            data[entry_off + 7],
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
    let entry_off = s.line_offset + idx * LINE_ENTRY_SIZE;
    if entry_off + LINE_ENTRY_SIZE > data.len() {
        return None;
    }

    let file_off = u32::from_le_bytes([
        data[entry_off + 8],
        data[entry_off + 9],
        data[entry_off + 10],
        data[entry_off + 11],
    ]) as usize;
    let line_num = u32::from_le_bytes([
        data[entry_off + 12],
        data[entry_off + 13],
        data[entry_off + 14],
        data[entry_off + 15],
    ]);

    let file_start = s.strings_offset + file_off;
    let file = read_nul_str(data, file_start)?;

    Some((file, line_num))
}

/// Read a NUL-terminated string from the data at the given offset.
fn read_nul_str(data: &[u8], offset: usize) -> Option<&str> {
    if offset >= data.len() {
        return None;
    }
    let remaining = &data[offset..];
    let nul_pos = remaining.iter().position(|&b| b == 0)?;
    core::str::from_utf8(&remaining[..nul_pos]).ok()
}
