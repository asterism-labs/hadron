# hadron-mmio-macros

Proc-macro companion for `hadron-mmio`. Provides the `register_block!` macro that generates typed, safe MMIO register accessor structs from a declarative `[offset; width; access_mode] name` DSL.
