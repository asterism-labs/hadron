//! Code generation for items emitted unconditionally (no feature gate).
//!
//! Generates: syscall number constants, error constants, `#[repr(C)]` structs,
//! named constants, `Syscall` enum, `SyscallGroup` enum, and compile-time tests.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::model::SyscallDefs;

/// Generate all common (always-emitted) items.
pub(crate) fn generate(defs: &SyscallDefs) -> TokenStream {
    let errors = gen_errors(defs);
    let types = gen_types(defs);
    let constants = gen_constants(defs);
    let syscall_consts = gen_syscall_constants(defs);
    let group_enum = gen_group_enum(defs);
    let syscall_enum = gen_syscall_enum(defs);
    let tests = gen_tests(defs);

    quote! {
        #errors
        #types
        #constants
        #syscall_consts
        #group_enum
        #syscall_enum
        #tests
    }
}

fn gen_errors(defs: &SyscallDefs) -> TokenStream {
    let items: Vec<_> = defs
        .errors
        .iter()
        .map(|e| {
            let attrs = &e.attrs;
            let name = &e.name;
            let value = &e.value;
            quote! {
                #(#attrs)*
                pub const #name: isize = #value;
            }
        })
        .collect();
    quote! { #(#items)* }
}

fn gen_types(defs: &SyscallDefs) -> TokenStream {
    let items: Vec<_> = defs
        .types
        .iter()
        .map(|t| {
            let attrs = &t.attrs;
            let name = &t.name;
            let fields: Vec<_> = t
                .fields
                .iter()
                .map(|f| {
                    let fattrs = &f.attrs;
                    let fname = &f.name;
                    let fty = &f.ty;
                    quote! { #(#fattrs)* pub #fname: #fty, }
                })
                .collect();
            quote! {
                #(#attrs)*
                #[repr(C)]
                #[allow(clippy::pub_underscore_fields)]
                pub struct #name {
                    #(#fields)*
                }
            }
        })
        .collect();
    quote! { #(#items)* }
}

fn gen_constants(defs: &SyscallDefs) -> TokenStream {
    let items: Vec<_> = defs
        .constants
        .iter()
        .map(|c| {
            let attrs = &c.attrs;
            let name = &c.name;
            let ty = &c.ty;
            let value = &c.value;
            quote! {
                #(#attrs)*
                pub const #name: #ty = #value;
            }
        })
        .collect();
    quote! { #(#items)* }
}

fn gen_syscall_constants(defs: &SyscallDefs) -> TokenStream {
    let items: Vec<_> = defs
        .groups
        .iter()
        .flat_map(|g| {
            g.syscalls.iter().map(move |s| {
                let number = s.number(g.range_start);
                let const_name = format_ident!("SYS_{}", s.name.to_string().to_uppercase());
                let attrs = &s.attrs;

                if let Some(ref res) = s.reserved {
                    let phase = res.phase;
                    let reason = format!("reserved for Phase {phase}");
                    quote! {
                        #(#attrs)*
                        #[allow(dead_code, reason = #reason)]
                        pub const #const_name: usize = #number;
                    }
                } else {
                    quote! {
                        #(#attrs)*
                        pub const #const_name: usize = #number;
                    }
                }
            })
        })
        .collect();
    quote! { #(#items)* }
}

fn gen_group_enum(defs: &SyscallDefs) -> TokenStream {
    let variants: Vec<_> = defs
        .groups
        .iter()
        .map(|g| {
            let variant = to_pascal(&g.name.to_string());
            let variant_ident = format_ident!("{}", variant);
            let attrs = &g.attrs;
            quote! { #(#attrs)* #variant_ident }
        })
        .collect();

    let variant_names: Vec<_> = defs
        .groups
        .iter()
        .map(|g| {
            let variant = to_pascal(&g.name.to_string());
            let variant_ident = format_ident!("{}", variant);
            let name_str = g.name.to_string();
            quote! { Self::#variant_ident => #name_str }
        })
        .collect();

    quote! {
        /// Syscall group categories.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum SyscallGroup {
            #(#variants,)*
        }

        impl SyscallGroup {
            /// Return the group name as a string.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    #(#variant_names,)*
                }
            }
        }
    }
}

fn gen_syscall_enum(defs: &SyscallDefs) -> TokenStream {
    // Collect all syscalls with their metadata.
    let mut variants = Vec::new();
    let mut from_nr_arms = Vec::new();
    let mut nr_arms = Vec::new();
    let mut name_arms = Vec::new();
    let mut group_arms = Vec::new();
    let mut arg_count_arms = Vec::new();
    let mut is_reserved_arms = Vec::new();
    let mut args_arms = Vec::new();

    for group in &defs.groups {
        let group_variant = format_ident!("{}", to_pascal(&group.name.to_string()));
        for syscall in &group.syscalls {
            let variant = format_ident!("{}", to_pascal(&syscall.name.to_string()));
            let number = syscall.number(group.range_start);
            let name_str = syscall.name.to_string();
            let arg_count = syscall.args.len();
            let is_reserved = syscall.reserved.is_some();
            let attrs = &syscall.attrs;

            let arg_name_strs: Vec<_> = syscall.args.iter().map(|a| a.name.to_string()).collect();
            let arg_ty_strs: Vec<_> = syscall
                .args
                .iter()
                .map(|a| {
                    let ty = &a.ty;
                    quote!(#ty).to_string()
                })
                .collect();
            let arg_pair_tokens: Vec<_> = arg_name_strs
                .iter()
                .zip(arg_ty_strs.iter())
                .map(|(n, t)| quote! { (#n, #t) })
                .collect();

            variants.push(quote! { #(#attrs)* #variant });
            from_nr_arms.push(quote! { #number => Some(Self::#variant) });
            nr_arms.push(quote! { Self::#variant => #number });
            name_arms.push(quote! { Self::#variant => #name_str });
            group_arms.push(quote! { Self::#variant => SyscallGroup::#group_variant });
            arg_count_arms.push(quote! { Self::#variant => #arg_count });
            is_reserved_arms.push(quote! { Self::#variant => #is_reserved });
            args_arms.push(quote! {
                Self::#variant => &[#(#arg_pair_tokens),*]
            });
        }
    }

    quote! {
        /// Enum of all defined syscalls.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Syscall {
            #(#variants,)*
        }

        impl Syscall {
            /// Look up a syscall by its number.
            #[must_use]
            pub const fn from_nr(nr: usize) -> Option<Self> {
                match nr {
                    #(#from_nr_arms,)*
                    _ => None,
                }
            }

            /// Return this syscall's number.
            #[must_use]
            pub const fn nr(self) -> usize {
                match self {
                    #(#nr_arms,)*
                }
            }

            /// Return this syscall's name.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    #(#name_arms,)*
                }
            }

            /// Return this syscall's group.
            #[must_use]
            pub const fn group(self) -> SyscallGroup {
                match self {
                    #(#group_arms,)*
                }
            }

            /// Return the number of arguments this syscall takes.
            #[must_use]
            pub const fn arg_count(self) -> usize {
                match self {
                    #(#arg_count_arms,)*
                }
            }

            /// Return whether this syscall is reserved for a future phase.
            #[must_use]
            pub const fn is_reserved(self) -> bool {
                match self {
                    #(#is_reserved_arms,)*
                }
            }

            /// Return argument metadata as `(name, type)` string pairs.
            #[must_use]
            pub const fn args(self) -> &'static [(&'static str, &'static str)] {
                match self {
                    #(#args_arms,)*
                }
            }
        }
    }
}

fn gen_tests(defs: &SyscallDefs) -> TokenStream {
    // Collect all syscall numbers for uniqueness test.
    let mut number_checks = Vec::new();
    let mut all_numbers = Vec::new();
    let mut all_names = Vec::new();

    for group in &defs.groups {
        for syscall in &group.syscalls {
            let number = syscall.number(group.range_start);
            let name = syscall.name.to_string();
            all_numbers.push(number);
            all_names.push(name);
        }
    }

    for (i, (num_i, name_i)) in all_numbers.iter().zip(all_names.iter()).enumerate() {
        for (j, (num_j, name_j)) in all_numbers.iter().zip(all_names.iter()).enumerate() {
            if i < j && num_i == num_j {
                // This is caught by validation, but we also generate a compile-time assert.
                number_checks.push(quote! {
                    compile_error!(concat!(
                        "syscall number collision: ", #name_i, " and ", #name_j
                    ));
                });
            }
        }
    }

    // Error positivity checks.
    let error_checks: Vec<_> = defs
        .errors
        .iter()
        .map(|e| {
            let name = &e.name;
            let name_str = e.name.to_string();
            quote! {
                assert!(#name > 0, concat!("error code ", #name_str, " must be positive"));
            }
        })
        .collect();

    // Range checks: each syscall number is within its group range.
    let range_checks: Vec<_> = defs
        .groups
        .iter()
        .flat_map(|g| {
            let start = g.range_start;
            let end = g.range_end;
            let gname = g.name.to_string();
            g.syscalls.iter().map(move |s| {
                let const_name = format_ident!("SYS_{}", s.name.to_string().to_uppercase());
                let sname = s.name.to_string();
                quote! {
                    assert!(
                        #const_name >= #start && #const_name < #end,
                        concat!(
                            "syscall ", #sname, " (SYS_",
                            stringify!(#const_name),
                            ") is outside group `", #gname, "` range"
                        )
                    );
                }
            })
        })
        .collect();

    quote! {
        #(#number_checks)*

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn syscall_numbers_unique() {
                let numbers: &[usize] = &[
                    #(#all_numbers),*
                ];
                let names: &[&str] = &[
                    #(#all_names),*
                ];
                for i in 0..numbers.len() {
                    for j in (i + 1)..numbers.len() {
                        assert_ne!(
                            numbers[i], numbers[j],
                            "syscall number collision between `{}` and `{}`",
                            names[i], names[j]
                        );
                    }
                }
            }

            #[test]
            fn error_numbers_positive() {
                #(#error_checks)*
            }

            #[test]
            fn syscall_numbers_in_group_range() {
                #(#range_checks)*
            }
        }
    }
}

/// Convert a `snake_case` name to `PascalCase`.
fn to_pascal(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}
