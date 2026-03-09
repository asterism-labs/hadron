//! Code generation for the `register_block!` macro.
//!
//! Transforms the parsed register block definition into a struct with typed
//! MMIO accessor methods.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::parse::{AccessMode, RegisterBlock, RegisterDef};

/// Generates the complete output for a register block definition.
pub fn generate(block: &RegisterBlock) -> TokenStream {
    let vis = &block.vis;
    let name = &block.name;
    let attrs = &block.attrs;

    let methods: Vec<TokenStream> = block.registers.iter().map(generate_methods).collect();

    quote! {
        #(#attrs)*
        #vis struct #name {
            base: VirtAddr,
        }

        impl #name {
            /// Creates a new register block accessor.
            ///
            /// # Safety
            ///
            /// `base` must point to a valid, mapped MMIO region covering all
            /// defined registers.
            #vis unsafe fn new(base: VirtAddr) -> Self {
                Self { base }
            }

            /// Returns the base virtual address.
            #[must_use]
            #vis fn base(&self) -> VirtAddr {
                self.base
            }

            #(#methods)*
        }
    }
}

/// Generates accessor methods for a single register.
fn generate_methods(reg: &RegisterDef) -> TokenStream {
    let mut methods = TokenStream::new();

    let read_method = generate_read(reg);
    let write_method = generate_write(reg);

    if let Some(m) = read_method {
        methods.extend(m);
    }
    if let Some(m) = write_method {
        methods.extend(m);
    }

    methods
}

/// Generates the read accessor for a register, if applicable.
fn generate_read(reg: &RegisterDef) -> Option<TokenStream> {
    if reg.access == AccessMode::WriteOnly {
        return None;
    }

    let name = &reg.name;
    let offset = &reg.offset;
    let width_ty = width_type(reg);
    let attrs = &reg.attrs;

    if let Some(ref bf_type) = reg.bitflags_type {
        Some(quote! {
            #(#attrs)*
            #[inline]
            pub fn #name(&self) -> #bf_type {
                // SAFETY: Caller of `new` guarantees base points to a valid MMIO region.
                let raw = unsafe {
                    core::ptr::read_volatile(
                        (self.base.as_u64() + #offset) as *const #width_ty
                    )
                };
                #bf_type::from_bits_retain(raw)
            }
        })
    } else {
        Some(quote! {
            #(#attrs)*
            #[inline]
            pub fn #name(&self) -> #width_ty {
                // SAFETY: Caller of `new` guarantees base points to a valid MMIO region.
                unsafe {
                    core::ptr::read_volatile(
                        (self.base.as_u64() + #offset) as *const #width_ty
                    )
                }
            }
        })
    }
}

/// Generates the write accessor for a register, if applicable.
fn generate_write(reg: &RegisterDef) -> Option<TokenStream> {
    if reg.access == AccessMode::ReadOnly {
        return None;
    }

    let name = &reg.name;
    let setter_name = format_ident!("set_{}", name);
    let offset = &reg.offset;
    let width_ty = width_type(reg);
    let attrs = &reg.attrs;

    // Build doc comment for setter.
    let set_doc = format!("Writes the `{}` register.", name);

    if let Some(ref bf_type) = reg.bitflags_type {
        Some(quote! {
            #[doc = #set_doc]
            #[inline]
            pub fn #setter_name(&self, value: #bf_type) {
                // SAFETY: Caller of `new` guarantees base points to a valid MMIO region.
                unsafe {
                    core::ptr::write_volatile(
                        (self.base.as_u64() + #offset) as *mut #width_ty,
                        value.bits(),
                    );
                }
            }
        })
    } else {
        Some(quote! {
            #[doc = #set_doc]
            #[inline]
            pub fn #setter_name(&self, value: #width_ty) {
                // SAFETY: Caller of `new` guarantees base points to a valid MMIO region.
                unsafe {
                    core::ptr::write_volatile(
                        (self.base.as_u64() + #offset) as *mut #width_ty,
                        value,
                    );
                }
            }
        })
    }
}

/// Returns the token stream for the register's width type.
fn width_type(reg: &RegisterDef) -> TokenStream {
    let type_name = reg.width.type_name();
    let ident = format_ident!("{}", type_name);
    quote! { #ident }
}
