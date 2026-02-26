# hadron-mmio

Typed MMIO register block abstractions for the Hadron kernel. This `no_std`
crate provides the `register_block!` macro, which generates safe, strongly-typed
accessor structs for memory-mapped I/O registers from a concise declarative
definition. All `unsafe` volatile access is consolidated into the struct's
`new()` constructor, so individual register reads and writes are safe by
construction.

## Features

- **Declarative DSL** -- define registers with `[offset; width; access_mode]`
  syntax (e.g., `[0x04; u32; rw] ghc`), and the macro generates the full
  accessor struct.
- **Access mode enforcement** -- `ro` registers only generate readers, `wo`
  registers only generate writers, and `rw` registers generate both, preventing
  invalid operations at compile time.
- **Width safety** -- supports `u8`, `u16`, `u32`, and `u64` register widths
  with correctly-sized volatile reads and writes.
- **Optional bitflags integration** -- registers can map to a bitflags type via
  `=> Type` syntax, automatically calling `from_bits_retain()` / `.bits()` on
  access.
- **Single unsafe boundary** -- the only `unsafe` call is in `new()`, which
  takes a raw base pointer; all generated field accessors are safe.
