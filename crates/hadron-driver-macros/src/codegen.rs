//! Code generation for the `#[hadron_driver(...)]` attribute macro.
//!
//! Generates:
//! 1. A per-driver context struct with fields for declared capabilities only
//! 2. `HasCapability` impls for each declared capability
//! 3. `device()` method for PCI drivers
//! 4. The original impl block with `DriverContext` replaced
//! 5. A wrapper probe function adapting the full probe context
//! 6. A linker-section static entry (behind `#[cfg(target_os = "none")]`)

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::ItemImpl;

use crate::parse::{Capability, DriverDef, DriverKind};

/// Main entry point: generates all code from the parsed definition and impl block.
pub fn generate(def: DriverDef, impl_block: ItemImpl) -> syn::Result<TokenStream> {
    let struct_name = extract_struct_name(&impl_block)?;
    let ctx_name = format_ident!("{}Context", struct_name);

    let ctx_struct = gen_context_struct(&def, &ctx_name);
    let has_cap_impls = gen_has_capability_impls(&def, &ctx_name);
    let device_method = gen_device_method(&def, &ctx_name);
    let rewritten_impl = gen_rewritten_impl(&impl_block, &ctx_name)?;
    let wrapper_fn = gen_wrapper_fn(&def, &struct_name, &ctx_name);
    let linker_entry = gen_linker_entry(&def, &struct_name);

    Ok(quote! {
        #ctx_struct
        #has_cap_impls
        #device_method
        #rewritten_impl
        #wrapper_fn
        #linker_entry
    })
}

/// Extracts the struct name from `impl StructName { ... }`.
fn extract_struct_name(impl_block: &ItemImpl) -> syn::Result<Ident> {
    if let syn::Type::Path(type_path) = &*impl_block.self_ty {
        if let Some(segment) = type_path.path.segments.last() {
            return Ok(segment.ident.clone());
        }
    }
    Err(syn::Error::new_spanned(
        &impl_block.self_ty,
        "expected a simple struct name (e.g., `impl AhciDriver`)",
    ))
}

/// Path prefix for `hadron_kernel::driver_api::capability::`.
fn cap_path() -> TokenStream {
    quote!(hadron_kernel::driver_api::capability)
}

/// Generates the per-driver context struct.
fn gen_context_struct(def: &DriverDef, ctx_name: &Ident) -> TokenStream {
    let cap_mod = cap_path();

    let mut fields = Vec::new();

    // PCI drivers get a `device` field.
    if def.kind == DriverKind::Pci {
        fields.push(quote! {
            device: hadron_kernel::driver_api::pci::PciDeviceInfo
        });
    }

    // One field per declared capability.
    for cap in &def.capabilities {
        let field = cap.field_ident();
        let ty = cap.type_ident();
        fields.push(quote! { #field: #cap_mod::#ty });
    }

    quote! {
        /// Generated driver context with compile-time capability enforcement.
        #[cfg(target_os = "none")]
        pub struct #ctx_name {
            #(#fields),*
        }
    }
}

/// Generates `HasCapability<T>` impls for each declared capability.
fn gen_has_capability_impls(def: &DriverDef, ctx_name: &Ident) -> TokenStream {
    let cap_mod = cap_path();

    let impls: Vec<_> = def
        .capabilities
        .iter()
        .map(|cap| {
            let ty = cap.type_ident();
            let field = cap.field_ident();
            quote! {
                #[cfg(target_os = "none")]
                impl #cap_mod::HasCapability<#cap_mod::#ty> for #ctx_name {
                    fn get(&self) -> &#cap_mod::#ty {
                        &self.#field
                    }
                }
            }
        })
        .collect();

    quote! { #(#impls)* }
}

/// Generates the `device()` method for PCI drivers.
fn gen_device_method(def: &DriverDef, ctx_name: &Ident) -> TokenStream {
    if def.kind != DriverKind::Pci {
        return TokenStream::new();
    }

    quote! {
        #[cfg(target_os = "none")]
        impl #ctx_name {
            /// Returns information about the matched PCI device.
            pub fn device(&self) -> &hadron_kernel::driver_api::pci::PciDeviceInfo {
                &self.device
            }
        }
    }
}

/// Rewrites the impl block, replacing `DriverContext` with the generated context type.
///
/// The rewritten impl block is gated behind `#[cfg(target_os = "none")]` because
/// probe functions reference kernel-only APIs.
fn gen_rewritten_impl(impl_block: &ItemImpl, ctx_name: &Ident) -> syn::Result<TokenStream> {
    // Convert the impl block to a token stream and replace DriverContext.
    let tokens = quote!(#impl_block);
    let source = tokens.to_string();
    let replaced = source.replace("DriverContext", &ctx_name.to_string());
    let replaced_tokens: TokenStream = replaced
        .parse()
        .map_err(|e| syn::Error::new(Span::call_site(), format!("failed to reparse impl block: {e}")))?;
    Ok(quote! {
        #[cfg(target_os = "none")]
        #replaced_tokens
    })
}

/// Generates the wrapper probe function.
fn gen_wrapper_fn(def: &DriverDef, struct_name: &Ident, ctx_name: &Ident) -> TokenStream {
    let cap_mod = cap_path();
    let wrapper_name = format_ident!(
        "__{}_probe_wrapper",
        to_snake_case(&struct_name.to_string())
    );

    // Build field initializers for the context struct.
    let mut field_inits = Vec::new();
    if def.kind == DriverKind::Pci {
        field_inits.push(quote! { device: __ctx.device });
    }
    for cap in &def.capabilities {
        let field = cap.field_ident();
        let src_field = cap.probe_context_field();
        field_inits.push(quote! { #field: __ctx.#src_field });
    }

    match def.kind {
        DriverKind::Pci => {
            quote! {
                #[cfg(target_os = "none")]
                fn #wrapper_name(
                    __ctx: hadron_kernel::driver_api::probe_context::PciProbeContext,
                ) -> Result<
                    hadron_kernel::driver_api::registration::PciDriverRegistration,
                    hadron_kernel::driver_api::error::DriverError,
                > {
                    use #cap_mod::CapabilityAccess;
                    let ctx = #ctx_name { #(#field_inits),* };
                    #struct_name::probe(ctx)
                }
            }
        }
        DriverKind::Platform => {
            quote! {
                #[cfg(target_os = "none")]
                fn #wrapper_name(
                    __ctx: hadron_kernel::driver_api::probe_context::PlatformProbeContext,
                ) -> Result<
                    hadron_kernel::driver_api::registration::PlatformDriverRegistration,
                    hadron_kernel::driver_api::error::DriverError,
                > {
                    use #cap_mod::CapabilityAccess;
                    let ctx = #ctx_name { #(#field_inits),* };
                    #struct_name::probe(ctx)
                }
            }
        }
    }
}

/// Generates the linker-section static entry.
fn gen_linker_entry(def: &DriverDef, struct_name: &Ident) -> TokenStream {
    let cap_mod = cap_path();
    let driver_name = &def.name;
    let wrapper_name = format_ident!(
        "__{}_probe_wrapper",
        to_snake_case(&struct_name.to_string())
    );
    let entry_name = format_ident!(
        "__{}_{}_ENTRY",
        struct_name.to_string().to_uppercase(),
        match def.kind {
            DriverKind::Pci => "PCI",
            DriverKind::Platform => "PLATFORM",
        }
    );

    // Build the capability flags expression.
    let flags_expr = if def.capabilities.is_empty() {
        quote! { #cap_mod::CapabilityFlags::empty() }
    } else {
        let flag_idents: Vec<_> = def
            .capabilities
            .iter()
            .map(|cap| {
                let flag = cap.flag_ident();
                quote! { #cap_mod::CapabilityFlags::#flag }
            })
            .collect();
        quote! {
            #cap_mod::CapabilityFlags::from_bits_truncate(
                #(#flag_idents.bits())|*
            )
        }
    };

    match def.kind {
        DriverKind::Pci => {
            let pci_ids = def.pci_ids.as_ref().expect("validated in parse");
            quote! {
                #[cfg(target_os = "none")]
                #[used]
                #[unsafe(link_section = ".hadron_pci_drivers")]
                static #entry_name: hadron_kernel::driver_api::registration::PciDriverEntry =
                    hadron_kernel::driver_api::registration::PciDriverEntry {
                        name: #driver_name,
                        id_table: #pci_ids,
                        capabilities: #flags_expr,
                        probe: #wrapper_name,
                    };
            }
        }
        DriverKind::Platform => {
            let compatible = def.compatible.as_ref().expect("validated in parse");
            quote! {
                #[cfg(target_os = "none")]
                #[used]
                #[unsafe(link_section = ".hadron_platform_drivers")]
                static #entry_name: hadron_kernel::driver_api::registration::PlatformDriverEntry =
                    hadron_kernel::driver_api::registration::PlatformDriverEntry {
                        name: #driver_name,
                        compatible: #compatible,
                        capabilities: #flags_expr,
                        init: #wrapper_name,
                    };
            }
        }
    }
}

/// Converts `PascalCase` to `snake_case`.
fn to_snake_case(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        } else {
            result.push(ch);
        }
    }
    result
}
