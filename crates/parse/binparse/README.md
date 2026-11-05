# hadron-binparse

Safe binary data parsing primitives for `no_std` environments. Provides the `FromBytes` trait for reading `#[repr(C)]` structs from raw byte buffers, a `BinaryReader` cursor for sequential reads, and a `FixedEntryIter` for iterating over uniform-sized entries. Re-exports `#[derive(FromBytes)]` and `#[derive(TableEntries)]` from `hadron-binparse-macros`.

## Features

- `FromBytes` trait with `read_from` and `read_at` for safe, unaligned reads from byte slices
- Built-in implementations for all primitive integer types (`u8`..`u64`, `i8`..`i64`) and fixed-size byte arrays
- `BinaryReader` cursor that tracks position and provides sequential `read`, `skip`, and `remaining` operations
- `FixedEntryIter` for iterating over arrays of fixed-size `FromBytes` entries with exact-size hints
- `#[derive(FromBytes)]` macro with compile-time `#[repr(C)]` verification and field-type assertions
- `#[derive(TableEntries)]` macro for generating TLV-style (type-length-value) entry iterators from enums
