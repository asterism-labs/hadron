//! Parsing logic for `#[kernel_test(...)]` attribute arguments.

use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, LitStr, Token};

/// Test stage — controls when during boot the test runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestStage {
    EarlyBoot,
    BeforeExecutor,
    WithExecutor,
    Userspace,
}

/// Inclusive range for concurrent test instances.
#[derive(Debug, Clone, Copy)]
pub struct InstanceRange {
    pub start: u32,
    pub end_inclusive: u32,
}

/// Parsed `#[kernel_test(...)]` attribute.
pub struct KernelTestDef {
    pub stage: TestStage,
    pub instances: Option<InstanceRange>,
    pub timeout: Option<u32>,
}

impl Parse for KernelTestDef {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut stage = None;
        let mut instances = None;
        let mut timeout = None;

        // Empty attribute defaults to early_boot.
        if input.is_empty() {
            return Ok(Self {
                stage: TestStage::EarlyBoot,
                instances: None,
                timeout: None,
            });
        }

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "stage" => {
                    let value: LitStr = input.parse()?;
                    stage = Some(match value.value().as_str() {
                        "early_boot" => TestStage::EarlyBoot,
                        "before_executor" => TestStage::BeforeExecutor,
                        "with_executor" => TestStage::WithExecutor,
                        "userspace" => TestStage::Userspace,
                        _ => {
                            return Err(syn::Error::new(
                                value.span(),
                                "expected one of: \"early_boot\", \"before_executor\", \
                                 \"with_executor\", \"userspace\"",
                            ));
                        }
                    });
                }
                "instances" => {
                    let start: LitInt = input.parse()?;
                    input.parse::<Token![..=]>()?;
                    let end: LitInt = input.parse()?;
                    let start_val: u32 = start.base10_parse()?;
                    let end_val: u32 = end.base10_parse()?;
                    if end_val < start_val {
                        return Err(syn::Error::new(
                            end.span(),
                            "instance range end must be >= start",
                        ));
                    }
                    instances = Some(InstanceRange {
                        start: start_val,
                        end_inclusive: end_val,
                    });
                }
                "timeout" => {
                    let lit: LitInt = input.parse()?;
                    let val: u32 = lit.base10_parse()?;
                    if val == 0 {
                        return Err(syn::Error::new(
                            lit.span(),
                            "timeout must be greater than 0",
                        ));
                    }
                    timeout = Some(val);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown attribute `{key}`; expected one of: stage, instances, timeout"
                        ),
                    ));
                }
            }

            // Consume trailing comma if present.
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self {
            stage: stage.unwrap_or(TestStage::EarlyBoot),
            instances,
            timeout,
        })
    }
}
