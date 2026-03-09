//! Proc-macro crate for the `register_block!` MMIO register DSL.
//!
//! Generates typed, safe MMIO register accessors from a declarative definition.
//! The single `unsafe` point is at struct construction (`new()`); all generated
//! read/write methods are safe.

mod codegen;
mod parse;

use proc_macro::TokenStream;
use syn::parse_macro_input;

use crate::parse::RegisterBlock;

/// Generates a typed MMIO register block struct with safe accessors.
///
/// # Syntax
///
/// ```ignore
/// register_block! {
///     /// Doc comment for the struct.
///     pub StructName {
///         /// Doc comment for the register.
///         [offset; width; access_mode] name => OptionalBitflagsType,
///     }
/// }
/// ```
///
/// - `offset` — byte offset from base (integer literal, e.g. `0x04`)
/// - `width` — `u8`, `u16`, `u32`, or `u64`
/// - `access_mode` — `ro` (read-only), `wo` (write-only), `rw` (read-write)
/// - `name` — register name (generates method names)
/// - `=> Type` — optional bitflags type (must have `from_bits_retain`/`.bits()`)
///
/// # Generated Code
///
/// For each register, generates:
/// - `ro`/`rw`: `fn name(&self) -> Type` (reader)
/// - `wo`/`rw`: `fn set_name(&self, value: Type)` (writer)
///
/// # Example
///
/// ```ignore
/// use hadron_mmio::register_block;
///
/// register_block! {
///     /// AHCI HBA generic host control registers.
///     pub AhciHbaRegs {
///         /// Host Capabilities (read-only).
///         [0x00; u32; ro] cap => HbaCap,
///         /// Global Host Control.
///         [0x04; u32; rw] ghc => HbaGhc,
///         /// Interrupt Status.
///         [0x08; u32; rw] is,
///         /// Ports Implemented (read-only).
///         [0x0C; u32; ro] pi,
///     }
/// }
/// ```
#[proc_macro]
pub fn register_block(input: TokenStream) -> TokenStream {
    let block = parse_macro_input!(input as RegisterBlock);
    codegen::generate(&block).into()
}
