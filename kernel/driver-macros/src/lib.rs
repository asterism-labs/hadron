//! Proc-macro crate for the `#[hadron_driver(...)]` attribute.
//!
//! Generates per-driver context types with compile-time capability enforcement,
//! wrapper functions, and linker-section entries from a declarative driver
//! definition.

mod codegen;
mod parse;

use proc_macro::TokenStream;
use syn::parse_macro_input;

use parse::DriverDef;

/// Declares a kernel driver with typed capability enforcement.
///
/// # PCI driver example
///
/// ```ignore
/// struct AhciDriver;
///
/// #[hadron_driver(
///     name = "ahci",
///     kind = pci,
///     capabilities = [Irq, Mmio, Dma, PciConfig],
///     pci_ids = &ID_TABLE,
/// )]
/// impl AhciDriver {
///     fn probe(ctx: DriverContext) -> Result<PciDriverRegistration, DriverError> {
///         let mmio = ctx.capability::<MmioCapability>();
///         // ...
///     }
/// }
/// ```
///
/// # Platform driver example
///
/// ```ignore
/// struct Uart16550Driver;
///
/// #[hadron_driver(
///     name = "uart16550",
///     kind = platform,
///     capabilities = [Irq, Spawner],
///     compatible = "ns16550",
/// )]
/// impl Uart16550Driver {
///     fn probe(ctx: DriverContext) -> Result<PlatformDriverRegistration, DriverError> {
///         let irq = ctx.capability::<IrqCapability>();
///         // ...
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn hadron_driver(attr: TokenStream, item: TokenStream) -> TokenStream {
    let def = parse_macro_input!(attr as DriverDef);
    let impl_block = parse_macro_input!(item as syn::ItemImpl);

    match codegen::generate(def, impl_block) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
