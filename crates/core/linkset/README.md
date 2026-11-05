# hadron-linkset

Safe, declarative macros for reading typed data from linker sections. This
`no_std` crate encapsulates the unsafe patterns needed to access
linker-section-bounded data behind safe APIs.

## Features

- `declare_linkset!` -- declares a function returning a typed `&'static [T]`
  slice from a linker section bounded by `__<section>_start` / `__<section>_end`
  symbols.
- `linkset_entry!` -- places a typed `#[used]` static into the matching linker
  section.
- `declare_linkset_blob!` -- declares a function returning a raw
  `&'static [u8]` slice from a linker section, for binary blobs such as HKIF
  data.
- Zero dependencies -- pure `no_std` macro-only crate with no runtime cost.
- Used by the driver framework to collect `#[hadron_driver]` registration
  entries at link time without manual bookkeeping.
