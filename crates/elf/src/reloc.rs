//! ELF64 relocation parsing and computation.
//!
//! Provides zero-copy, zero-allocation parsing of `SHT_RELA` relocation entries
//! and pure-arithmetic relocation value computation for the x86-64 architecture.

use crate::header::{le_i64, le_u64};
use core::fmt;

// ---------------------------------------------------------------------------
// x86-64 relocation type constants (ELF ABI supplement)
// ---------------------------------------------------------------------------

/// No relocation.
pub const R_X86_64_NONE: u32 = 0;

/// Absolute 64-bit: `S + A`.
pub const R_X86_64_64: u32 = 1;

/// PC-relative 32-bit: `S + A - P`.
pub const R_X86_64_PC32: u32 = 2;

/// PLT-relative 32-bit: `S + A - P` (same as `PC32` for static linking).
pub const R_X86_64_PLT32: u32 = 4;

/// Global data: `S` (symbol value).
pub const R_X86_64_GLOB_DAT: u32 = 6;

/// Base-relative 64-bit: `B + A` (used in static-PIE / `ET_DYN`).
pub const R_X86_64_RELATIVE: u32 = 8;

/// Absolute 32-bit, zero-extended: `S + A`.
pub const R_X86_64_32: u32 = 10;

/// Absolute 32-bit, sign-extended: `S + A`.
pub const R_X86_64_32S: u32 = 11;

// ---------------------------------------------------------------------------
// Size of a RELA entry
// ---------------------------------------------------------------------------

/// Size of an ELF64 `Rela` entry (24 bytes).
const ELF64_RELA_SIZE: usize = 24;

// ---------------------------------------------------------------------------
// Elf64Rela
// ---------------------------------------------------------------------------

/// A parsed ELF64 relocation entry with addend (`SHT_RELA`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elf64Rela {
    /// Offset within the section (or virtual address) where the relocation applies.
    pub r_offset: u64,
    /// Relocation type (lower 32 bits of `r_info`).
    pub r_type: u32,
    /// Symbol table index (upper 32 bits of `r_info`).
    pub r_sym: u32,
    /// Addend value.
    pub r_addend: i64,
}

impl Elf64Rela {
    /// Parse a single Rela entry from raw bytes at the given offset.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "r_info split into r_type/r_sym is defined by the ELF spec"
    )]
    fn parse(data: &[u8], offset: usize) -> Self {
        let b = &data[offset..];
        let r_offset = le_u64(b, 0);
        let r_info = le_u64(b, 8);
        let r_addend = le_i64(b, 16);
        Self {
            r_offset,
            r_type: r_info as u32,        // lower 32 bits
            r_sym: (r_info >> 32) as u32, // upper 32 bits
            r_addend,
        }
    }
}

// ---------------------------------------------------------------------------
// RelaIter
// ---------------------------------------------------------------------------

/// An iterator over ELF64 `Rela` entries in a section.
pub struct RelaIter<'a> {
    data: &'a [u8],
    offset: usize,
    end: usize,
}

impl<'a> RelaIter<'a> {
    /// Creates a new iterator over Rela entries.
    ///
    /// `data` is the full ELF file; `offset` and `end` delimit the section
    /// containing the Rela entries.
    pub(crate) fn new(data: &'a [u8], offset: usize, end: usize) -> Self {
        Self { data, offset, end }
    }
}

impl Iterator for RelaIter<'_> {
    type Item = Elf64Rela;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset + ELF64_RELA_SIZE > self.end {
            return None;
        }
        let rela = Elf64Rela::parse(self.data, self.offset);
        self.offset += ELF64_RELA_SIZE;
        Some(rela)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.end - self.offset) / ELF64_RELA_SIZE;
        (remaining, Some(remaining))
    }
}

// ---------------------------------------------------------------------------
// RelocValue / RelocError
// ---------------------------------------------------------------------------

/// The computed value to write at the relocation target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocValue {
    /// A 32-bit value to write.
    U32(u32),
    /// A 64-bit value to write.
    U64(u64),
}

/// Errors from relocation computation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocError {
    /// The relocation type is not supported.
    UnsupportedType(u32),
    /// The computed value overflows the target field width.
    Overflow,
}

impl fmt::Display for RelocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedType(ty) => write!(f, "unsupported relocation type {ty}"),
            Self::Overflow => write!(f, "relocation value overflow"),
        }
    }
}

// ---------------------------------------------------------------------------
// compute_x86_64_reloc
// ---------------------------------------------------------------------------

/// Computes the relocation value for an x86-64 relocation entry.
///
/// This is pure arithmetic — no memory access or side effects.
///
/// # Parameters
///
/// - `rela`: The relocation entry.
/// - `sym_value`: Resolved symbol value (0 for `R_X86_64_RELATIVE`).
/// - `base_addr`: Load base address (for `R_X86_64_RELATIVE` in `ET_DYN`).
/// - `place_addr`: Virtual address where the relocation is written (`P`).
///
/// # Returns
///
/// A tuple of `(target_offset, value)` where `target_offset` is `rela.r_offset`
/// and `value` is the computed [`RelocValue`] to write at that location.
///
/// # Errors
///
/// Returns [`RelocError::UnsupportedType`] for unknown relocation types and
/// [`RelocError::Overflow`] if the value doesn't fit the target width.
// Relocation arithmetic intentionally uses wrapping casts between signed/unsigned
// types and truncations with explicit overflow checks — these are defined by the
// ELF x86-64 ABI.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
pub fn compute_x86_64_reloc(
    rela: &Elf64Rela,
    sym_value: u64,
    base_addr: u64,
    place_addr: u64,
) -> Result<(u64, RelocValue), RelocError> {
    let s = sym_value;
    let a = rela.r_addend;
    let p = place_addr;
    let b = base_addr;

    let value = match rela.r_type {
        R_X86_64_NONE => return Ok((rela.r_offset, RelocValue::U64(0))),

        // S + A (64-bit)
        R_X86_64_64 => {
            let result = s.wrapping_add(a as u64);
            RelocValue::U64(result)
        }

        // S + A - P (32-bit, sign-extended)
        R_X86_64_PC32 | R_X86_64_PLT32 => {
            let result = (s as i64).wrapping_add(a).wrapping_sub(p as i64);
            // Must fit in i32
            let truncated = result as i32;
            if i64::from(truncated) != result {
                return Err(RelocError::Overflow);
            }
            RelocValue::U32(truncated as u32)
        }

        // S (64-bit)
        R_X86_64_GLOB_DAT => RelocValue::U64(s),

        // B + A (64-bit, PIE base-relative)
        R_X86_64_RELATIVE => {
            let result = b.wrapping_add(a as u64);
            RelocValue::U64(result)
        }

        // S + A (32-bit, zero-extended)
        R_X86_64_32 => {
            let result = s.wrapping_add(a as u64);
            // Must fit in u32
            if result > u64::from(u32::MAX) {
                return Err(RelocError::Overflow);
            }
            RelocValue::U32(result as u32)
        }

        // S + A (32-bit, sign-extended)
        R_X86_64_32S => {
            let result = (s as i64).wrapping_add(a);
            let truncated = result as i32;
            if i64::from(truncated) != result {
                return Err(RelocError::Overflow);
            }
            RelocValue::U32(truncated as u32)
        }

        other => return Err(RelocError::UnsupportedType(other)),
    };

    Ok((rela.r_offset, value))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 24-byte Rela entry.
    fn make_rela(r_offset: u64, r_sym: u32, r_type: u32, r_addend: i64) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[0..8].copy_from_slice(&r_offset.to_le_bytes());
        let r_info = (u64::from(r_sym) << 32) | u64::from(r_type);
        b[8..16].copy_from_slice(&r_info.to_le_bytes());
        b[16..24].copy_from_slice(&r_addend.to_le_bytes());
        b
    }

    #[test]
    fn parse_rela_entry() {
        let data = make_rela(0x1000, 5, R_X86_64_64, -42);
        let rela = Elf64Rela::parse(&data, 0);
        assert_eq!(rela.r_offset, 0x1000);
        assert_eq!(rela.r_sym, 5);
        assert_eq!(rela.r_type, R_X86_64_64);
        assert_eq!(rela.r_addend, -42);
    }

    #[test]
    fn rela_iter_multiple() {
        let mut data = Vec::new();
        data.extend_from_slice(&make_rela(0x100, 1, R_X86_64_64, 0));
        data.extend_from_slice(&make_rela(0x200, 2, R_X86_64_RELATIVE, 8));
        data.extend_from_slice(&make_rela(0x300, 0, R_X86_64_NONE, 0));

        let iter = RelaIter::new(&data, 0, data.len());
        let entries: Vec<_> = iter.collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].r_offset, 0x100);
        assert_eq!(entries[1].r_offset, 0x200);
        assert_eq!(entries[2].r_offset, 0x300);
    }

    #[test]
    fn rela_iter_empty() {
        let data = [0u8; 0];
        let iter = RelaIter::new(&data, 0, 0);
        assert_eq!(iter.count(), 0);
    }

    #[test]
    fn rela_iter_partial_entry_ignored() {
        // 30 bytes = 1 full entry (24) + 6 leftover
        let mut data = vec![0u8; 30];
        data[0..24].copy_from_slice(&make_rela(0x100, 1, R_X86_64_64, 0));
        let entries: Vec<_> = RelaIter::new(&data, 0, data.len()).collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn rela_iter_size_hint() {
        let mut data = Vec::new();
        data.extend_from_slice(&make_rela(0, 0, 0, 0));
        data.extend_from_slice(&make_rela(0, 0, 0, 0));
        let iter = RelaIter::new(&data, 0, data.len());
        assert_eq!(iter.size_hint(), (2, Some(2)));
    }

    // --- compute_x86_64_reloc tests ---

    #[test]
    fn reloc_none() {
        let rela = Elf64Rela {
            r_offset: 0x100,
            r_type: R_X86_64_NONE,
            r_sym: 0,
            r_addend: 0,
        };
        let (off, val) = compute_x86_64_reloc(&rela, 0, 0, 0).unwrap();
        assert_eq!(off, 0x100);
        assert_eq!(val, RelocValue::U64(0));
    }

    #[test]
    fn reloc_64() {
        // S + A = 0x1000 + 0x10 = 0x1010
        let rela = Elf64Rela {
            r_offset: 0x200,
            r_type: R_X86_64_64,
            r_sym: 1,
            r_addend: 0x10,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x1000, 0, 0).unwrap();
        assert_eq!(val, RelocValue::U64(0x1010));
    }

    #[test]
    fn reloc_64_negative_addend() {
        // S + A = 0x1000 + (-8) = 0xFF8
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_64,
            r_sym: 1,
            r_addend: -8,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x1000, 0, 0).unwrap();
        assert_eq!(val, RelocValue::U64(0xFF8));
    }

    #[test]
    fn reloc_pc32() {
        // S + A - P = 0x2000 + (-4) - 0x1000 = 0xFFC
        let rela = Elf64Rela {
            r_offset: 0x1000,
            r_type: R_X86_64_PC32,
            r_sym: 1,
            r_addend: -4,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x2000, 0, 0x1000).unwrap();
        assert_eq!(val, RelocValue::U32(0xFFC));
    }

    #[test]
    fn reloc_pc32_negative_result() {
        // S + A - P = 0x1000 + (-4) - 0x2000 = -0x1004 (fits in i32)
        let rela = Elf64Rela {
            r_offset: 0x2000,
            r_type: R_X86_64_PC32,
            r_sym: 1,
            r_addend: -4,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x1000, 0, 0x2000).unwrap();
        assert_eq!(val, RelocValue::U32((-0x1004_i32) as u32));
    }

    #[test]
    fn reloc_pc32_overflow() {
        // Huge distance that doesn't fit in i32
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_PC32,
            r_sym: 1,
            r_addend: 0,
        };
        let result = compute_x86_64_reloc(&rela, 0x1_0000_0000, 0, 0);
        assert_eq!(result, Err(RelocError::Overflow));
    }

    #[test]
    fn reloc_plt32() {
        // Same formula as PC32: S + A - P
        let rela = Elf64Rela {
            r_offset: 0x100,
            r_type: R_X86_64_PLT32,
            r_sym: 1,
            r_addend: -4,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x2000, 0, 0x100).unwrap();
        assert_eq!(val, RelocValue::U32(0x1EFC));
    }

    #[test]
    fn reloc_glob_dat() {
        // S = 0x3000
        let rela = Elf64Rela {
            r_offset: 0x400,
            r_type: R_X86_64_GLOB_DAT,
            r_sym: 1,
            r_addend: 0,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x3000, 0, 0).unwrap();
        assert_eq!(val, RelocValue::U64(0x3000));
    }

    #[test]
    fn reloc_relative() {
        // B + A = 0x40_0000 + 0x1234 = 0x40_1234
        let rela = Elf64Rela {
            r_offset: 0x500,
            r_type: R_X86_64_RELATIVE,
            r_sym: 0,
            r_addend: 0x1234,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0, 0x40_0000, 0).unwrap();
        assert_eq!(val, RelocValue::U64(0x40_1234));
    }

    #[test]
    fn reloc_relative_negative_addend() {
        // B + A = 0x40_0000 + (-0x100) = 0x3F_FF00
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_RELATIVE,
            r_sym: 0,
            r_addend: -0x100,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0, 0x40_0000, 0).unwrap();
        assert_eq!(val, RelocValue::U64(0x3F_FF00));
    }

    #[test]
    fn reloc_32_zero_ext() {
        // S + A = 0x1000 + 0x10 = 0x1010, fits in u32
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_32,
            r_sym: 1,
            r_addend: 0x10,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x1000, 0, 0).unwrap();
        assert_eq!(val, RelocValue::U32(0x1010));
    }

    #[test]
    fn reloc_32_overflow() {
        // S + A > u32::MAX
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_32,
            r_sym: 1,
            r_addend: 0,
        };
        let result = compute_x86_64_reloc(&rela, 0x1_0000_0000, 0, 0);
        assert_eq!(result, Err(RelocError::Overflow));
    }

    #[test]
    fn reloc_32s_sign_ext() {
        // S + A = 0x1000 + (-8) = 0xFF8, fits in i32
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_32S,
            r_sym: 1,
            r_addend: -8,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0x1000, 0, 0).unwrap();
        assert_eq!(val, RelocValue::U32(0xFF8));
    }

    #[test]
    fn reloc_32s_negative_fits() {
        // S + A = 0 + (-1) = -1, fits in i32
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_32S,
            r_sym: 0,
            r_addend: -1,
        };
        let (_, val) = compute_x86_64_reloc(&rela, 0, 0, 0).unwrap();
        assert_eq!(val, RelocValue::U32((-1_i32) as u32));
    }

    #[test]
    fn reloc_32s_overflow() {
        // S + A > i32::MAX
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: R_X86_64_32S,
            r_sym: 1,
            r_addend: 0,
        };
        let result = compute_x86_64_reloc(&rela, 0x1_0000_0000, 0, 0);
        assert_eq!(result, Err(RelocError::Overflow));
    }

    #[test]
    fn reloc_unsupported_type() {
        let rela = Elf64Rela {
            r_offset: 0,
            r_type: 99,
            r_sym: 0,
            r_addend: 0,
        };
        let result = compute_x86_64_reloc(&rela, 0, 0, 0);
        assert_eq!(result, Err(RelocError::UnsupportedType(99)));
    }

    #[test]
    fn reloc_error_display() {
        let msg = format!("{}", RelocError::UnsupportedType(42));
        assert!(msg.contains("42"));
        let msg = format!("{}", RelocError::Overflow);
        assert!(msg.contains("overflow"));
    }
}
