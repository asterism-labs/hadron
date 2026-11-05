# hadron-dwarf

A minimal, zero-allocation DWARF `.debug_line` parser for the Hadron kernel. Provides zero-copy parsing of DWARF v4 and v5 line number programs, yielding address-to-source-line mappings for use in kernel stack trace symbolication. Contains no unsafe code.

## Features

- `DebugLine` iterator over compilation unit line programs within a `.debug_line` section
- `LineProgramHeader` parser supporting DWARF v4 (NUL-terminated tables) and v5 (structured format with content type/form pairs)
- `LineProgramIter` state machine that executes standard, extended, and special opcodes to emit `LineRow` entries
- File and directory table access with version-transparent indexing (1-based for v4, 0-based for v5)
- LEB128 decoder for both unsigned (ULEB128) and signed (SLEB128) variable-length integers
