# hadron-binparse-macros

Proc-macro companion for `hadron-binparse`. Provides `#[derive(FromBytes)]` for generating safe `FromBytes` implementations on `#[repr(C)]` structs with compile-time layout verification, and `#[derive(TableEntries)]` for generating TLV-style entry iterators over variable-length binary table entries (as used by ACPI MADT, SRAT, and similar tables).
