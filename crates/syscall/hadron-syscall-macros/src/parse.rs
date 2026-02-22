//! `syn::Parse` implementations for the syscall DSL.

use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Attribute, Ident, LitInt, Token, Type, braced, parenthesized};

use crate::model::*;

impl Parse for SyscallDefs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut errors = Vec::new();
        let mut types = Vec::new();
        let mut constants = Vec::new();
        let mut groups = Vec::new();

        while !input.is_empty() {
            let attrs = input.call(Attribute::parse_outer)?;
            let ident: Ident = input.parse()?;

            match ident.to_string().as_str() {
                "errors" => {
                    let content;
                    braced!(content in input);
                    while !content.is_empty() {
                        let err_attrs = content.call(Attribute::parse_outer)?;
                        let name: Ident = content.parse()?;
                        content.parse::<Token![=]>()?;
                        let value: LitInt = content.parse()?;
                        content.parse::<Token![;]>()?;
                        errors.push(ErrorDef {
                            attrs: err_attrs,
                            name,
                            value,
                        });
                    }
                }
                "types" => {
                    let content;
                    braced!(content in input);
                    while !content.is_empty() {
                        types.push(content.call(parse_type_def)?);
                    }
                }
                "constants" => {
                    let content;
                    braced!(content in input);
                    while !content.is_empty() {
                        let const_attrs = content.call(Attribute::parse_outer)?;
                        let name: Ident = content.parse()?;
                        content.parse::<Token![:]>()?;
                        let ty: Type = content.parse()?;
                        content.parse::<Token![=]>()?;
                        let value: syn::Expr = content.parse()?;
                        content.parse::<Token![;]>()?;
                        constants.push(ConstDef {
                            attrs: const_attrs,
                            name,
                            ty,
                            value,
                        });
                    }
                }
                "group" => {
                    groups.push(parse_group(attrs, input)?);
                }
                other => {
                    return Err(syn::Error::new(
                        ident.span(),
                        format!(
                            "expected `errors`, `types`, `constants`, or `group`, found `{other}`"
                        ),
                    ));
                }
            }
        }

        Ok(SyscallDefs {
            errors,
            types,
            constants,
            groups,
        })
    }
}

fn parse_type_def(input: ParseStream<'_>) -> syn::Result<TypeDef> {
    let attrs = input.call(Attribute::parse_outer)?;
    input.parse::<Token![struct]>()?;
    let name: Ident = input.parse()?;
    let content;
    braced!(content in input);

    let mut fields = Vec::new();
    while !content.is_empty() {
        let field_attrs = content.call(Attribute::parse_outer)?;
        let field_name: Ident = content.parse()?;
        content.parse::<Token![:]>()?;
        let ty: Type = content.parse()?;
        content.parse::<Token![,]>()?;
        fields.push(FieldDef {
            attrs: field_attrs,
            name: field_name,
            ty,
        });
    }

    Ok(TypeDef {
        attrs,
        name,
        fields,
    })
}

fn parse_group(outer_attrs: Vec<Attribute>, input: ParseStream<'_>) -> syn::Result<GroupDef> {
    let name: Ident = input.parse()?;

    let range;
    parenthesized!(range in input);
    let range_start: LitInt = range.parse()?;
    range.parse::<Token![..]>()?;
    let range_end: LitInt = range.parse()?;

    let content;
    braced!(content in input);

    let mut syscalls = Vec::new();
    while !content.is_empty() {
        syscalls.push(content.call(parse_syscall)?);
    }

    Ok(GroupDef {
        attrs: outer_attrs,
        name,
        range_start: range_start.base10_parse()?,
        range_end: range_end.base10_parse()?,
        syscalls,
    })
}

fn parse_syscall(input: ParseStream<'_>) -> syn::Result<SyscallDef> {
    let attrs = input.call(Attribute::parse_outer)?;
    let span = input.span();

    // Check for #[reserved(phase = N)]
    let mut reserved = None;
    let mut remaining_attrs = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("reserved") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("phase") {
                    meta.input.parse::<Token![=]>()?;
                    let lit: LitInt = meta.input.parse()?;
                    reserved = Some(ReservedInfo {
                        phase: lit.base10_parse()?,
                    });
                    Ok(())
                } else {
                    Err(meta.error("expected `phase`"))
                }
            })?;
        } else {
            remaining_attrs.push(attr);
        }
    }

    input.parse::<Token![fn]>()?;
    let name: Ident = input.parse()?;

    let args_content;
    parenthesized!(args_content in input);
    let args_punctuated: Punctuated<(Ident, Type), Token![,]> =
        args_content.parse_terminated(parse_arg, Token![,])?;
    let args: Vec<ArgDef> = args_punctuated
        .into_iter()
        .map(|(name, ty)| ArgDef { name, ty })
        .collect();

    input.parse::<Token![=]>()?;
    let offset_lit: LitInt = input.parse()?;
    let offset: usize = offset_lit.base10_parse()?;
    input.parse::<Token![;]>()?;

    Ok(SyscallDef {
        attrs: remaining_attrs,
        name,
        args,
        offset,
        reserved,
        span,
    })
}

fn parse_arg(input: ParseStream<'_>) -> syn::Result<(Ident, Type)> {
    let name: Ident = input.parse()?;
    input.parse::<Token![:]>()?;
    let ty: Type = input.parse()?;
    Ok((name, ty))
}
