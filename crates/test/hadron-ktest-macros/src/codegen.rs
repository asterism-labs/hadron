//! Code generation for the `#[kernel_test(...)]` attribute macro.
//!
//! Generates:
//! 1. The original test function (gated behind `#[cfg(ktest)]`)
//! 2. An async wrapper function (for async tests)
//! 3. A linker-section entry via `hadron_linkset::linkset_entry!`

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::ItemFn;

use crate::parse::{KernelTestDef, TestStage};

/// Main entry point: generates all code from the parsed definition and function.
pub fn generate(def: KernelTestDef, func: ItemFn) -> syn::Result<TokenStream> {
    let fn_name = &func.sig.ident;
    let is_async = func.sig.asyncness.is_some();
    let has_instances = def.instances.is_some();

    // Validate: sync stages cannot be async.
    if is_async && matches!(def.stage, TestStage::EarlyBoot | TestStage::BeforeExecutor) {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "early_boot and before_executor tests must be synchronous (not async)",
        ));
    }

    // Validate: instances require async.
    if has_instances && !is_async {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "instanced tests must be async",
        ));
    }

    // Validate: instances require a `&TestContext` parameter.
    if has_instances && func.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "instanced tests must accept `ctx: &TestContext` as a parameter",
        ));
    }

    // Validate: non-instanced tests should have no parameters.
    if !has_instances && !func.sig.inputs.is_empty() {
        return Err(syn::Error::new_spanned(
            &func.sig,
            "non-instanced tests must not have parameters",
        ));
    }

    let stage_tokens = gen_stage(&def.stage);
    let kind_tokens = gen_kind(is_async, has_instances);

    let (instance_start, instance_end) = match def.instances {
        Some(range) => (range.start, range.end_inclusive),
        None => (0, 0),
    };

    let static_name = gen_static_name(fn_name);
    let fn_name_str = fn_name.to_string();

    // Generate the wrapper and function pointer expression based on test kind.
    let (wrapper, fn_ptr_expr) = if !is_async {
        // Sync test: function pointer directly.
        (quote! {}, quote! { #fn_name as *const () })
    } else if !has_instances {
        // Async test without instances: generate a wrapper that returns a pinned future.
        let wrapper_name = format_ident!("__ktest_async_{}", fn_name);
        let wrapper = quote! {
            #[cfg(ktest)]
            fn #wrapper_name() -> ::core::pin::Pin<
                ::alloc::boxed::Box<dyn ::core::future::Future<Output = ()> + Send>
            > {
                ::alloc::boxed::Box::pin(#fn_name())
            }
        };
        (wrapper, quote! { #wrapper_name as *const () })
    } else {
        // Async instanced test: generate a wrapper that takes &'static TestContext.
        let wrapper_name = format_ident!("__ktest_instanced_{}", fn_name);
        let wrapper = quote! {
            #[cfg(ktest)]
            fn #wrapper_name(
                ctx: &'static hadron_ktest::TestContext,
            ) -> ::core::pin::Pin<
                ::alloc::boxed::Box<dyn ::core::future::Future<Output = ()> + Send>
            > {
                ::alloc::boxed::Box::pin(#fn_name(ctx))
            }
        };
        (wrapper, quote! { #wrapper_name as *const () })
    };

    Ok(quote! {
        #[cfg(ktest)]
        #func

        #wrapper

        #[cfg(ktest)]
        hadron_linkset::linkset_entry!("hadron_kernel_tests",
            #static_name: hadron_ktest::KernelTestDescriptor =
                hadron_ktest::KernelTestDescriptor {
                    name: #fn_name_str,
                    module_path: module_path!(),
                    stage: #stage_tokens,
                    kind: #kind_tokens,
                    instance_start: #instance_start,
                    instance_end_inclusive: #instance_end,
                    test_fn: #fn_ptr_expr,
                }
        );
    })
}

/// Generates the `TestStage` variant tokens.
fn gen_stage(stage: &TestStage) -> TokenStream {
    match stage {
        TestStage::EarlyBoot => quote! { hadron_ktest::TestStage::EarlyBoot },
        TestStage::BeforeExecutor => quote! { hadron_ktest::TestStage::BeforeExecutor },
        TestStage::WithExecutor => quote! { hadron_ktest::TestStage::WithExecutor },
        TestStage::Userspace => quote! { hadron_ktest::TestStage::Userspace },
    }
}

/// Generates the `TestKind` variant tokens.
fn gen_kind(is_async: bool, has_instances: bool) -> TokenStream {
    if !is_async {
        quote! { hadron_ktest::TestKind::Sync }
    } else if !has_instances {
        quote! { hadron_ktest::TestKind::Async }
    } else {
        quote! { hadron_ktest::TestKind::AsyncInstanced }
    }
}

/// Generates a unique static name from the function name.
fn gen_static_name(fn_name: &Ident) -> Ident {
    let upper = fn_name.to_string().to_uppercase();
    Ident::new(&format!("__KTEST_{upper}"), Span::call_site())
}
