//! Proc-macro crate for the `#[kernel_test(...)]` attribute.
//!
//! Generates linkset entries for kernel test descriptors, enabling staged
//! test execution during kernel boot.

mod codegen;
mod parse;

use proc_macro::TokenStream;
use syn::parse_macro_input;

use parse::KernelTestDef;

/// Marks a function as a kernel test, collected via linker sections.
///
/// # Stages
///
/// - `early_boot` (default) — runs after CPU, HHDM, PMM, VMM, and heap init
/// - `before_executor` — runs after ACPI, PCI, drivers, VFS, and logging init
/// - `with_executor` — runs inside the async executor as a spawned task
/// - `userspace` — runs with full kernel including userspace support
///
/// # Examples
///
/// ```ignore
/// #[kernel_test]
/// fn test_basic_alloc() {
///     let b = alloc::boxed::Box::new(42u64);
///     assert_eq!(*b, 42);
/// }
///
/// #[kernel_test(stage = "with_executor")]
/// async fn test_async_sleep() {
///     // ...
/// }
///
/// #[kernel_test(stage = "with_executor", instances = 0..=3)]
/// async fn test_concurrent(ctx: &hadron_ktest::TestContext) {
///     ctx.barrier().await;
///     // all 4 instances synchronized here
/// }
/// ```
#[proc_macro_attribute]
pub fn kernel_test(attr: TokenStream, item: TokenStream) -> TokenStream {
    let def = parse_macro_input!(attr as KernelTestDef);
    let func = parse_macro_input!(item as syn::ItemFn);

    match codegen::generate(def, func) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
