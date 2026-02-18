//! Proc-macro crate for the Hadron syscall definition DSL.
//!
//! Provides `define_syscalls!` which generates constants, types, enums,
//! a kernel dispatch trait, and userspace wrappers from a single definition.

mod gen_common;
mod gen_kernel;
mod gen_userspace;
mod model;
mod parse;
mod validate;

use proc_macro::TokenStream;
use syn::parse_macro_input;

use model::SyscallDefs;

/// Define all syscalls, error codes, types, and constants from a single DSL.
///
/// See `crates/hadron-syscall/src/lib.rs` for the full invocation.
#[proc_macro]
pub fn define_syscalls(input: TokenStream) -> TokenStream {
    let defs = parse_macro_input!(input as SyscallDefs);

    if let Err(errors) = validate::validate(&defs) {
        let mut combined = proc_macro2::TokenStream::new();
        for err in errors {
            combined.extend(err.to_compile_error());
        }
        return combined.into();
    }

    let common = gen_common::generate(&defs);
    let kernel = gen_kernel::generate(&defs);
    let userspace = gen_userspace::generate(&defs);

    let output = quote::quote! {
        #common
        #kernel
        #userspace
    };

    output.into()
}
