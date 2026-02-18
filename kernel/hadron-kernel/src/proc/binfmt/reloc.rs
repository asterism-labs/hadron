//! Kernel-side relocation application for ELF binaries.
//!
//! Applies `.rela.dyn` relocations for static-PIE (`ET_DYN`) binaries by
//! resolving symbols, computing values via [`hadron_elf::compute_x86_64_reloc`],
//! and writing the results into already-mapped user pages via the HHDM.

use hadron_core::addr::VirtAddr;
use hadron_core::mm::address_space::AddressSpace;
use hadron_core::mm::mapper::{PageMapper, PageTranslator};
use hadron_core::paging::Size4KiB;
use hadron_elf::{
    Elf64SectionHeader, ElfFile, RelocValue, SHN_UNDEF, SHT_DYNSYM, SHT_SYMTAB,
    compute_x86_64_reloc,
};

use super::BinaryError;

/// Applies `.rela.dyn` relocations for a static-PIE binary.
///
/// Iterates over all `SHT_RELA` sections in the ELF, resolves symbols from
/// the ELF's own symbol table, computes relocation values, and writes them
/// into the user address space via the HHDM.
///
/// # Errors
///
/// Returns [`BinaryError::RelocError`] if a relocation type is unsupported or
/// overflows, or [`BinaryError::ParseError`] if symbol resolution or address
/// translation fails.
pub fn apply_dyn_relocations<M: PageMapper<Size4KiB> + PageTranslator>(
    address_space: &AddressSpace<M>,
    elf: &ElfFile<'_>,
    base_addr: u64,
    hhdm_offset: u64,
) -> Result<(), BinaryError> {
    // Find the symbol table — try .dynsym first, fall back to .symtab.
    let symtab = elf
        .find_section_by_type(SHT_DYNSYM)
        .or_else(|| elf.find_section_by_type(SHT_SYMTAB));

    for rela_shdr in elf.rela_sections() {
        let Some(rela_iter) = elf.rela_entries(&rela_shdr) else {
            continue;
        };

        for rela in rela_iter {
            // Resolve symbol value.
            let sym_value = resolve_symbol(elf, symtab.as_ref(), rela.r_sym, base_addr)?;

            // The address where the relocation is applied (in virtual address space).
            let place_addr = base_addr + rela.r_offset;

            // Compute the relocation value (pure arithmetic).
            let (_, value) = compute_x86_64_reloc(&rela, sym_value, base_addr, place_addr)
                .map_err(BinaryError::RelocError)?;

            // Write the value into the mapped page via HHDM.
            write_reloc_value(address_space, place_addr, value, hhdm_offset)?;
        }
    }

    Ok(())
}

/// Resolves a symbol's value for relocation.
///
/// For symbol index 0 (no symbol, e.g. `R_X86_64_RELATIVE`), returns 0.
/// For defined symbols, returns `st_value + base_addr`.
/// For undefined symbols (`SHN_UNDEF`), returns an error.
fn resolve_symbol(
    elf: &ElfFile<'_>,
    symtab: Option<&Elf64SectionHeader>,
    sym_index: u32,
    base_addr: u64,
) -> Result<u64, BinaryError> {
    // Symbol index 0 means no symbol — used by R_X86_64_RELATIVE.
    if sym_index == 0 {
        return Ok(0);
    }

    let symtab_shdr = symtab
        .ok_or(BinaryError::ParseError("relocation references symbol but no symbol table"))?;

    let sym = elf
        .symbols(symtab_shdr)
        .and_then(|mut iter| iter.nth(sym_index as usize))
        .ok_or(BinaryError::ParseError("relocation symbol index out of range"))?;

    if sym.st_shndx == SHN_UNDEF {
        // Undefined symbol — for static-PIE this shouldn't happen; the binary
        // should be fully resolved. Report an error.
        return Err(BinaryError::ParseError(
            "relocation references undefined symbol in static-PIE",
        ));
    }

    // Defined symbol: value is relative to load base in ET_DYN.
    Ok(sym.st_value + base_addr)
}

/// Writes a relocation value into the user address space via HHDM.
///
/// Translates the virtual address to physical, then writes through the HHDM
/// mapping.
fn write_reloc_value<M: PageMapper<Size4KiB> + PageTranslator>(
    address_space: &AddressSpace<M>,
    vaddr: u64,
    value: RelocValue,
    hhdm_offset: u64,
) -> Result<(), BinaryError> {
    let phys = address_space
        .translate(VirtAddr::new(vaddr))
        .ok_or(BinaryError::ParseError("relocation target address not mapped"))?;

    let hhdm_ptr = (hhdm_offset + phys.as_u64()) as *mut u8;

    match value {
        RelocValue::U32(v) => {
            // SAFETY: The page at `phys` was just allocated and mapped by the
            // segment mapper. Writing 4 bytes within a mapped page via HHDM is
            // safe. The address space is not yet loaded into CR3, so no TLB
            // concerns.
            unsafe {
                core::ptr::write_unaligned(hhdm_ptr.cast::<u32>(), v);
            }
        }
        RelocValue::U64(v) => {
            // SAFETY: Same as above — 8 bytes within a mapped page via HHDM.
            unsafe {
                core::ptr::write_unaligned(hhdm_ptr.cast::<u64>(), v);
            }
        }
    }

    Ok(())
}
