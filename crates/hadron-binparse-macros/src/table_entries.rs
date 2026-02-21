//! Parse and code generation for `#[derive(TableEntries)]`.
//!
//! Generates an iterator over variable-length entries in a byte buffer,
//! where each entry has a type byte and a length byte as its first two
//! bytes (TLV-style), as used by ACPI MADT and similar tables.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Expr, Fields, Ident, Lit, Meta, Variant};

/// Parsed metadata from `#[table_entries(...)]` on the enum.
struct TableEntriesAttrs {
    /// Type of the type_field (e.g. `u8`).
    type_field_ty: Ident,
    /// Type of the length_field (e.g. `u8`).
    length_field_ty: Ident,
}

/// Parsed metadata from `#[entry(...)]` on a variant.
struct EntryVariantAttrs {
    /// The type ID value for this entry.
    type_id: u64,
    /// Minimum length (in bytes) for this entry.
    min_length: usize,
}

/// Parsed metadata from `#[field(...)]` on a variant field.
struct FieldAttrs {
    /// Byte offset within the entry data.
    offset: usize,
}

/// A parsed entry variant with its fields.
struct ParsedVariant {
    /// The variant identity.
    ident: Ident,
    /// Entry attributes.
    attrs: EntryVariantAttrs,
    /// Named fields with their offsets and types.
    fields: Vec<ParsedField>,
}

/// A parsed field within an entry variant.
struct ParsedField {
    /// Field name.
    ident: Ident,
    /// Field type.
    ty: syn::Type,
    /// Byte offset within the entry.
    offset: usize,
}

/// Generates the `TableEntries` derive output.
pub fn derive(input: &DeriveInput) -> syn::Result<TokenStream> {
    let enum_name = &input.ident;

    // Parse `#[table_entries(type_field = u8, length_field = u8)]`.
    let table_attrs = parse_table_entries_attrs(input)?;

    // Must be an enum.
    let data_enum = match &input.data {
        Data::Enum(e) => e,
        _ => {
            return Err(syn::Error::new_spanned(
                enum_name,
                "TableEntries can only be derived for enums",
            ));
        }
    };

    // Parse each variant.
    let mut entry_variants = Vec::new();
    let mut fallback_variant: Option<&Variant> = None;

    for variant in &data_enum.variants {
        if has_attr(&variant.attrs, "fallback") {
            fallback_variant = Some(variant);
            continue;
        }

        if has_attr(&variant.attrs, "entry") {
            let attrs = parse_entry_attrs(variant)?;
            let fields = parse_variant_fields(variant)?;
            entry_variants.push(ParsedVariant {
                ident: variant.ident.clone(),
                attrs,
                fields,
            });
        }
    }

    let iter_name = format_ident!("{}Iter", enum_name);

    // Generate match arms for each entry type.
    let match_arms: Vec<TokenStream> = entry_variants
        .iter()
        .map(|v| {
            let variant_ident = &v.ident;
            let type_id = v.attrs.type_id as u8;
            let min_length = v.attrs.min_length;

            let field_reads: Vec<TokenStream> = v
                .fields
                .iter()
                .map(|f| {
                    let field_ident = &f.ident;
                    let field_ty = &f.ty;
                    let offset = f.offset;
                    quote! {
                        #field_ident: <#field_ty as hadron_binparse::FromBytes>::read_at(
                            entry_data, #offset
                        ).unwrap_or_default()
                    }
                })
                .collect();

            quote! {
                #type_id if length >= #min_length => #enum_name::#variant_ident {
                    #(#field_reads),*
                }
            }
        })
        .collect();

    // Generate fallback arm.
    let fallback_arm = if let Some(fb) = fallback_variant {
        let fb_ident = &fb.ident;
        // The fallback variant has `entry_type` and `length` fields.
        quote! {
            // length is guaranteed <= 255 because it was read from a u8 field.
            #[expect(
                clippy::cast_possible_truncation,
                reason = "entry length fits in u8"
            )]
            _ => #enum_name::#fb_ident {
                entry_type,
                length: length as u8,
            }
        }
    } else {
        quote! { _ => return None }
    };

    Ok(quote! {
        /// Iterator over entries in a byte buffer.
        pub struct #iter_name<'a> {
            data: &'a [u8],
            pos: usize,
        }

        impl #enum_name {
            /// Creates an iterator over entries in the given byte slice.
            #[must_use]
            pub fn iter(data: &[u8]) -> #iter_name<'_> {
                #iter_name { data, pos: 0 }
            }
        }

        impl<'a> Iterator for #iter_name<'a> {
            type Item = #enum_name;

            fn next(&mut self) -> Option<Self::Item> {
                let remaining = self.data.get(self.pos..)?;

                // Each entry has at least a 2-byte header: type + length.
                if remaining.len() < 2 {
                    return None;
                }

                let entry_type = remaining[0];
                let length = remaining[1] as usize;

                if length < 2 || length > remaining.len() {
                    return None;
                }

                let entry_data = &remaining[..length];

                let entry = match entry_type {
                    #(#match_arms,)*
                    #fallback_arm
                };

                self.pos += length;
                Some(entry)
            }
        }
    })
}

/// Checks if an attribute list contains a given attribute name.
fn has_attr(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|a| a.path().is_ident(name))
}

/// Parses `#[table_entries(type_field = u8, length_field = u8)]`.
fn parse_table_entries_attrs(input: &DeriveInput) -> syn::Result<TableEntriesAttrs> {
    let attr = input
        .attrs
        .iter()
        .find(|a| a.path().is_ident("table_entries"))
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &input.ident,
                "TableEntries requires #[table_entries(type_field = u8, length_field = u8)]",
            )
        })?;

    let mut type_field_ty = None;
    let mut length_field_ty = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("type_field") {
            meta.input.parse::<syn::Token![=]>()?;
            type_field_ty = Some(meta.input.parse::<Ident>()?);
        } else if meta.path.is_ident("length_field") {
            meta.input.parse::<syn::Token![=]>()?;
            length_field_ty = Some(meta.input.parse::<Ident>()?);
        }
        Ok(())
    })?;

    Ok(TableEntriesAttrs {
        type_field_ty: type_field_ty.ok_or_else(|| {
            syn::Error::new_spanned(attr, "missing type_field")
        })?,
        length_field_ty: length_field_ty.ok_or_else(|| {
            syn::Error::new_spanned(attr, "missing length_field")
        })?,
    })
}

/// Parses `#[entry(type_id = N, min_length = M)]` on a variant.
fn parse_entry_attrs(variant: &Variant) -> syn::Result<EntryVariantAttrs> {
    let attr = variant
        .attrs
        .iter()
        .find(|a| a.path().is_ident("entry"))
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &variant.ident,
                "entry variant requires #[entry(type_id = N, min_length = M)]",
            )
        })?;

    let mut type_id = None;
    let mut min_length = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("type_id") {
            meta.input.parse::<syn::Token![=]>()?;
            let lit: Lit = meta.input.parse()?;
            if let Lit::Int(lit_int) = lit {
                type_id = Some(lit_int.base10_parse::<u64>()?);
            }
        } else if meta.path.is_ident("min_length") {
            meta.input.parse::<syn::Token![=]>()?;
            let lit: Lit = meta.input.parse()?;
            if let Lit::Int(lit_int) = lit {
                min_length = Some(lit_int.base10_parse::<usize>()?);
            }
        }
        Ok(())
    })?;

    Ok(EntryVariantAttrs {
        type_id: type_id
            .ok_or_else(|| syn::Error::new_spanned(attr, "missing type_id"))?,
        min_length: min_length
            .ok_or_else(|| syn::Error::new_spanned(attr, "missing min_length"))?,
    })
}

/// Parses `#[field(offset = N)]` from a variant's named fields.
fn parse_variant_fields(variant: &Variant) -> syn::Result<Vec<ParsedField>> {
    let fields = match &variant.fields {
        Fields::Named(named) => &named.named,
        _ => {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "entry variants must have named fields",
            ));
        }
    };

    let mut parsed = Vec::new();
    for field in fields {
        let field_ident = field.ident.clone().unwrap();
        let field_ty = field.ty.clone();

        let offset_attr = field
            .attrs
            .iter()
            .find(|a| a.path().is_ident("field"))
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &field_ident,
                    "entry fields require #[field(offset = N)]",
                )
            })?;

        let mut offset = None;
        offset_attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("offset") {
                meta.input.parse::<syn::Token![=]>()?;
                let lit: Lit = meta.input.parse()?;
                if let Lit::Int(lit_int) = lit {
                    offset = Some(lit_int.base10_parse::<usize>()?);
                }
            }
            Ok(())
        })?;

        parsed.push(ParsedField {
            ident: field_ident.clone(),
            ty: field_ty,
            offset: offset.ok_or_else(|| {
                syn::Error::new_spanned(&field_ident, "missing offset in #[field()]")
            })?,
        });
    }

    Ok(parsed)
}
