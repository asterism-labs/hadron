# hadron-syscall-macros

Proc-macro companion for `hadron-syscall`. Provides the `define_syscalls!` macro that parses a declarative syscall DSL and generates constants, `#[repr(C)]` types, error codes, dispatch enums, kernel handler traits, and userspace assembly stubs from a single definition.
