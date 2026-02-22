//! Code generation for the `kernel` feature: `SyscallHandler` trait and `dispatch()` function.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::model::SyscallDefs;

/// Generate kernel-only items gated behind `#[cfg(feature = "kernel")]`.
pub(crate) fn generate(defs: &SyscallDefs) -> TokenStream {
    let trait_def = gen_handler_trait(defs);
    let dispatch_fn = gen_dispatch(defs);

    quote! {
        #[cfg(feature = "kernel")]
        #trait_def

        #[cfg(feature = "kernel")]
        #dispatch_fn
    }
}

fn gen_handler_trait(defs: &SyscallDefs) -> TokenStream {
    let mut methods = Vec::new();

    for group in &defs.groups {
        for syscall in &group.syscalls {
            let method_name = format_ident!("sys_{}", syscall.name);
            let attrs = &syscall.attrs;

            // Build parameter list: &self, then each arg as usize.
            let params: Vec<_> = syscall
                .args
                .iter()
                .map(|a| {
                    let name = &a.name;
                    quote! { #name: usize }
                })
                .collect();

            if let Some(ref reserved) = syscall.reserved {
                let phase = reserved.phase;
                let reason = format!("reserved for Phase {phase}");
                // Reserved syscalls get a default -ENOSYS implementation.
                methods.push(quote! {
                    #(#attrs)*
                    #[allow(unused_variables, reason = #reason)]
                    fn #method_name(&self, #(#params),*) -> isize {
                        -ENOSYS
                    }
                });
            } else {
                // Active syscalls are required (no default).
                methods.push(quote! {
                    #(#attrs)*
                    fn #method_name(&self, #(#params),*) -> isize;
                });
            }
        }
    }

    quote! {
        /// Trait for handling syscalls. Implement this to provide syscall dispatch.
        ///
        /// Active syscalls must be implemented. Reserved syscalls have default
        /// implementations returning `-ENOSYS`.
        pub trait SyscallHandler {
            #(#methods)*
        }
    }
}

fn gen_dispatch(defs: &SyscallDefs) -> TokenStream {
    let mut match_arms = Vec::new();

    for group in &defs.groups {
        for syscall in &group.syscalls {
            let method_name = format_ident!("sys_{}", syscall.name);
            let const_name = format_ident!("SYS_{}", syscall.name.to_string().to_uppercase());

            // Map argument positions to a0..a4.
            let arg_names: Vec<_> = (0..syscall.args.len())
                .map(|i| format_ident!("a{}", i))
                .collect();

            match_arms.push(quote! {
                #const_name => handler.#method_name(#(#arg_names),*)
            });
        }
    }

    quote! {
        /// Dispatch a syscall to the appropriate handler method.
        ///
        /// Returns `-ENOSYS` for unknown syscall numbers.
        pub fn dispatch<H: SyscallHandler>(
            handler: &H,
            nr: usize,
            a0: usize,
            a1: usize,
            a2: usize,
            a3: usize,
            a4: usize,
        ) -> isize {
            match nr {
                #(#match_arms,)*
                _ => -ENOSYS,
            }
        }
    }
}
