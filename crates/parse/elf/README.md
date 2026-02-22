# hadron-elf

A minimal, zero-allocation ELF64 parser for the Hadron kernel. Parses ELF64 headers, `PT_LOAD` segments, section headers, symbol tables, string tables, and `SHT_RELA` relocation entries from raw byte slices. Supports `ET_EXEC`, `ET_DYN`, and `ET_REL` file types targeting x86-64. Contains no unsafe code.

## Features

- Zero-copy `ElfFile` entry point with header validation (magic, class, endianness, machine type)
- `PT_LOAD` segment iteration with virtual address, data slice, memory size, and permission flags
- Section header iteration with lookup by type (`SHT_SYMTAB`, `SHT_RELA`, etc.) or by name
- Symbol table and string table parsing with `SymbolIter` and `StringTable`
- `SHT_RELA` relocation entry parsing with `RelaIter`
- Pure-arithmetic x86-64 relocation computation (`R_X86_64_64`, `PC32`, `PLT32`, `RELATIVE`, `32`, `32S`, `GLOB_DAT`) with overflow detection
