//! Address-to-symbol resolution using kernel ELF symbol tables.
//!
//! Reuses the `extract_symbols` infrastructure from the HBTF module.

use std::path::Path;

use crate::artifact::hbtf;

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

        let mut func_symbols = hbtf::extract_symbols(&elf, virt_base);
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
