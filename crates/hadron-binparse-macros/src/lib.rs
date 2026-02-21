//! Proc-macro crate for `#[derive(FromBytes)]` and `#[derive(TableEntries)]`.
//!
//! - `FromBytes` generates `unsafe impl hadron_binparse::FromBytes for T {}`
//!   with compile-time assertions verifying `#[repr(C)]` layout and that all
//!   field types implement `FromBytes`.
//! - `TableEntries` generates an iterator over variable-length TLV entries in
//!   a byte buffer, as used by ACPI MADT and similar tables.

mod table_entries;

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

/// Derives `hadron_binparse::FromBytes` for a `#[repr(C)]` struct.
///
/// # Requirements
///
/// - The struct must have `#[repr(C)]` or `#[repr(C, packed)]`.
/// - All fields must implement `FromBytes`.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone, Copy, FromBytes)]
/// #[repr(C, packed)]
/// pub struct SdtHeader {
///     pub signature: [u8; 4],
///     pub length: u32,
///     // ...
/// }
/// ```
#[proc_macro_derive(FromBytes)]
pub fn derive_from_bytes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derives a variable-length entry iterator for an enum.
///
/// Generates a `{EnumName}Iter` iterator struct and an `iter()` constructor
/// method on the enum that parses TLV-style (type-length-value) entries from
/// a byte slice.
///
/// # Attributes
///
/// - `#[table_entries(type_field = u8, length_field = u8)]` on the enum
/// - `#[entry(type_id = N, min_length = M)]` on each known variant
/// - `#[field(offset = N)]` on each field within a variant
/// - `#[fallback]` on a variant to catch unknown entry types
///
/// # Example
///
/// ```ignore
/// #[derive(TableEntries)]
/// #[table_entries(type_field = u8, length_field = u8)]
/// pub enum MadtEntry {
///     #[entry(type_id = 0, min_length = 8)]
///     LocalApic {
///         #[field(offset = 2)] acpi_processor_id: u8,
///         #[field(offset = 3)] apic_id: u8,
///         #[field(offset = 4)] flags: u32,
///     },
///     #[fallback]
///     Unknown { entry_type: u8, length: u8 },
/// }
/// ```
#[proc_macro_derive(TableEntries, attributes(table_entries, entry, field, fallback))]
pub fn derive_table_entries(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match table_entries::derive(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn derive_impl(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;

    // Verify #[repr(C)] or #[repr(C, packed)].
    let has_repr_c = input.attrs.iter().any(|attr| {
        if !attr.path().is_ident("repr") {
            return false;
        }
        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("C") {
                found = true;
            }
            Ok(())
        });
        found
    });

    if !has_repr_c {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "FromBytes requires #[repr(C)] or #[repr(C, packed)]",
        ));
    }

    // Only support structs.
    let fields = match &input.data {
        Data::Struct(data) => &data.fields,
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "FromBytes can only be derived for structs",
            ));
        }
    };

    // Generate compile-time assertions that all field types implement FromBytes.
    let field_assertions = match fields {
        Fields::Named(named) => named
            .named
            .iter()
            .map(|f| {
                let ty = &f.ty;
                let field_name = f.ident.as_ref().unwrap();
                let assert_name = quote::format_ident!(
                    "_AssertFromBytes_{}_{}",
                    name,
                    field_name
                );
                quote! {
                    #[doc(hidden)]
                    #[allow(non_camel_case_types, dead_code)]
                    struct #assert_name where #ty: hadron_binparse::FromBytes;
                }
            })
            .collect::<Vec<_>>(),
        Fields::Unnamed(unnamed) => unnamed
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let ty = &f.ty;
                let assert_name = quote::format_ident!("_AssertFromBytes_{}_{}", name, i);
                quote! {
                    #[doc(hidden)]
                    #[allow(non_camel_case_types, dead_code)]
                    struct #assert_name where #ty: hadron_binparse::FromBytes;
                }
            })
            .collect::<Vec<_>>(),
        Fields::Unit => Vec::new(),
    };

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    Ok(quote! {
        #(#field_assertions)*

        // SAFETY: The derive macro has verified:
        // 1. The struct has #[repr(C)] layout.
        // 2. All field types implement FromBytes (checked via where-clause assertions).
        // 3. The struct must also be Copy (enforced by the trait bound).
        unsafe impl #impl_generics hadron_binparse::FromBytes for #name #ty_generics #where_clause {}
    })
}
