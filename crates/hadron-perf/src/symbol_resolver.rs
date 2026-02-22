//! Address-to-symbol resolution using kernel ELF symbol tables.
//!
//! Extracts function symbols directly from the ELF binary using `hadron-elf`,
//! without depending on gluon's HBTF module.

use std::path::Path;

/// Resolves virtual addresses to function names using kernel ELF symbols.
pub struct SymbolResolver {
    symbols: Vec<ResolvedSymbol>,
}

struct ResolvedSymbol {
    addr: u64,
    size: u32,
    name: String,
}

impl SymbolResolver {
    /// Create a resolver by extracting symbols from a kernel ELF binary.
    pub fn from_kernel_elf(kernel_elf: &Path, _kernel_vbase: u64) -> Self {
        let elf_data = match std::fs::read(kernel_elf) {
            Ok(d) => d,
            Err(_) => return Self { symbols: Vec::new() },
        };

        let elf = match hadron_elf::ElfFile::parse(&elf_data) {
            Ok(e) => e,
            Err(_) => return Self { symbols: Vec::new() },
        };

        let virt_base = elf
            .load_segments()
            .map(|seg| seg.vaddr)
            .min()
            .unwrap_or(elf.entry_point());

        let mut func_symbols = extract_symbols(&elf, virt_base);
        func_symbols.sort_by_key(|s| s.addr);

        let symbols = func_symbols
            .into_iter()
            .map(|s| ResolvedSymbol {
                // Convert from offset-from-virt-base to absolute address.
                addr: s.addr + virt_base,
                size: s.size,
                name: s.name,
            })
            .collect();

        Self { symbols }
    }

    /// Create an empty resolver (no symbols available).
    #[allow(dead_code)] // useful for tests and when no kernel ELF is available
    pub fn empty() -> Self {
        Self { symbols: Vec::new() }
    }

    /// Resolve an absolute virtual address to a function name.
    ///
    /// Returns `None` if the address doesn't fall within any known function.
    pub fn resolve(&self, addr: u64) -> Option<String> {
        if self.symbols.is_empty() {
            return None;
        }

        // Binary search for the last symbol with addr <= target.
        let mut lo = 0usize;
        let mut hi = self.symbols.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.symbols[mid].addr <= addr {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        if lo == 0 {
            return None;
        }

        let sym = &self.symbols[lo - 1];
        let offset = addr - sym.addr;
        if sym.size > 0 && offset >= u64::from(sym.size) {
            return None;
        }

        Some(sym.name.clone())
    }
}

// ---------------------------------------------------------------------------
// Inline symbol extraction (from gluon's HBTF module)
// ---------------------------------------------------------------------------

struct FuncSymbol {
    /// Offset from kernel virtual base.
    addr: u64,
    /// Symbol size in bytes.
    size: u32,
    /// Demangled function name.
    name: String,
}

/// Extract function symbols from the ELF symbol table.
fn extract_symbols(elf: &hadron_elf::ElfFile<'_>, kernel_virt_base: u64) -> Vec<FuncSymbol> {
    let symtab = match elf.find_section_by_type(hadron_elf::SHT_SYMTAB) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let strtab = match elf.linked_strtab(&symtab) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let syms = match elf.symbols(&symtab) {
        Some(s) => s,
        None => return Vec::new(),
    };

    let mut result = Vec::new();

    for sym in syms {
        // Only include defined function symbols.
        if sym.sym_type() != hadron_elf::STT_FUNC {
            continue;
        }
        if sym.st_shndx == hadron_elf::SHN_UNDEF {
            continue;
        }
        if sym.st_value == 0 {
            continue;
        }
        // Skip symbols below the kernel virtual base.
        if sym.st_value < kernel_virt_base {
            continue;
        }

        let raw_name = match strtab.get(sym.st_name) {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };

        let demangled = format!("{:#}", rustc_demangle::demangle(raw_name));

        result.push(FuncSymbol {
            addr: sym.st_value.wrapping_sub(kernel_virt_base),
            size: sym.st_size as u32,
            name: demangled,
        });
    }

    result
}
