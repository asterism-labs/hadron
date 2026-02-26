//! Parsing logic for the `register_block!` DSL.
//!
//! Handles parsing of the DSL syntax into intermediate representation types
//! that the code generator can consume.

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Ident, LitInt, Token, Visibility, braced, bracketed};

/// A complete register block definition.
pub struct RegisterBlock {
    /// Doc attributes on the struct.
    pub attrs: Vec<Attribute>,
    /// Visibility of the generated struct.
    pub vis: Visibility,
    /// Name of the generated struct.
    pub name: Ident,
    /// Register definitions.
    pub registers: Vec<RegisterDef>,
}

/// Access mode for a register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// Read-only.
    ReadOnly,
    /// Write-only.
    WriteOnly,
    /// Read-write.
    ReadWrite,
}

/// Width of a register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegWidth {
    /// 8-bit register.
    U8,
    /// 16-bit register.
    U16,
    /// 32-bit register.
    U32,
    /// 64-bit register.
    U64,
}

impl RegWidth {
    /// Returns the Rust type name for this width.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
        }
    }
}

/// A single register definition.
pub struct RegisterDef {
    /// Doc attributes on this register.
    pub attrs: Vec<Attribute>,
    /// Byte offset from base.
    pub offset: LitInt,
    /// Register width.
    pub width: RegWidth,
    /// Access mode.
    pub access: AccessMode,
    /// Register name (used for method names).
    pub name: Ident,
    /// Optional associated bitflags type.
    pub bitflags_type: Option<Ident>,
}

impl Parse for RegisterBlock {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let name: Ident = input.parse()?;

        let content;
        braced!(content in input);

        let mut registers = Vec::new();
        while !content.is_empty() {
            registers.push(content.call(parse_register)?);
        }

        Ok(Self {
            attrs,
            vis,
            name,
            registers,
        })
    }
}

/// Parses a single register definition line.
fn parse_register(input: ParseStream) -> syn::Result<RegisterDef> {
    let attrs = input.call(Attribute::parse_outer)?;

    // Parse [offset; width; access_mode]
    let bracket_content;
    bracketed!(bracket_content in input);

    let offset: LitInt = bracket_content.parse()?;
    bracket_content.parse::<Token![;]>()?;

    let width_ident: Ident = bracket_content.parse()?;
    let width = match width_ident.to_string().as_str() {
        "u8" => RegWidth::U8,
        "u16" => RegWidth::U16,
        "u32" => RegWidth::U32,
        "u64" => RegWidth::U64,
        _ => {
            return Err(syn::Error::new(
                width_ident.span(),
                "expected register width: u8, u16, u32, or u64",
            ));
        }
    };

    bracket_content.parse::<Token![;]>()?;

    let access_ident: Ident = bracket_content.parse()?;
    let access = match access_ident.to_string().as_str() {
        "ro" => AccessMode::ReadOnly,
        "wo" => AccessMode::WriteOnly,
        "rw" => AccessMode::ReadWrite,
        _ => {
            return Err(syn::Error::new(
                access_ident.span(),
                "expected access mode: ro, wo, or rw",
            ));
        }
    };

    // Parse register name.
    let name: Ident = input.parse()?;

    // Parse optional `=> Type`.
    let bitflags_type = if input.peek(Token![=>]) {
        input.parse::<Token![=>]>()?;
        Some(input.parse::<Ident>()?)
    } else {
        None
    };

    // Consume trailing comma if present.
    let _ = input.parse::<Option<Token![,]>>();

    Ok(RegisterDef {
        attrs,
        offset,
        width,
        access,
        name,
        bitflags_type,
    })
}

/// Keyword for the `register_block!` macro entry point.
///
/// This is used internally by the proc-macro to identify the macro invocation.
pub mod kw {
    syn::custom_keyword!(register_block);
}
