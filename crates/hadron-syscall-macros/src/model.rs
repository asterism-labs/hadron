//! Intermediate representation for the syscall DSL.

use proc_macro2::Span;
use syn::{Attribute, Ident, LitInt, Type};

/// Top-level definition from the DSL.
pub(crate) struct SyscallDefs {
    pub errors: Vec<ErrorDef>,
    pub types: Vec<TypeDef>,
    pub constants: Vec<ConstDef>,
    pub groups: Vec<GroupDef>,
}

/// An error code definition: `ENOENT = 2;`
pub(crate) struct ErrorDef {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub value: LitInt,
}

/// A `#[repr(C)]` struct definition.
pub(crate) struct TypeDef {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub fields: Vec<FieldDef>,
}

/// A struct field.
pub(crate) struct FieldDef {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub ty: Type,
}

/// A named constant: `QUERY_MEMORY: u64 = 0;`
pub(crate) struct ConstDef {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub ty: Type,
    pub value: syn::Expr,
}

/// A syscall group: `group task(0x00..0x10) { ... }`
pub(crate) struct GroupDef {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub range_start: usize,
    pub range_end: usize,
    pub syscalls: Vec<SyscallDef>,
}

/// A single syscall: `fn task_exit(status: usize) = 0x00;`
pub(crate) struct SyscallDef {
    pub attrs: Vec<Attribute>,
    pub name: Ident,
    pub args: Vec<ArgDef>,
    pub offset: usize,
    pub reserved: Option<ReservedInfo>,
    pub span: Span,
}

/// Argument to a syscall.
pub(crate) struct ArgDef {
    pub name: Ident,
    pub ty: Type,
}

/// Metadata for `#[reserved(phase = N)]`.
pub(crate) struct ReservedInfo {
    pub phase: usize,
}

impl SyscallDef {
    /// Compute the absolute syscall number given the group's start.
    pub fn number(&self, group_start: usize) -> usize {
        group_start + self.offset
    }
}
