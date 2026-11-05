# hadron-fdt

A standalone, `no_std` Flattened Device Tree (DTB) parser. Provides zero-copy access to nodes, properties, and memory reservations from a raw DTB byte slice, with path-based node lookup and `compatible` string search.

## Features

- Header validation (magic, version, block bounds) with typed `FdtError` variants
- `FdtNode` with iterators over properties (`PropertyIter`) and direct children (`ChildIter`)
- Typed property accessors: `as_u32`, `as_u64`, `as_str`, and `as_str_list` for null-separated string lists
- Path-based node lookup (`find_node("/soc/serial@10000000")`) and recursive `compatible` search
- Memory reservation block iteration via `MemReservationIter`
- Big-endian primitive types (`Be32`, `Be64`) backed by `hadron-binparse::FromBytes`
