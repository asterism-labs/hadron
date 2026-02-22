//! Parsing logic for the `#[hadron_driver(...)]` attribute arguments.

use proc_macro2::Span;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Token};

/// The kind of driver: PCI or platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    Pci,
    Platform,
}

/// A capability declared in the `capabilities = [...]` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Irq,
    Mmio,
    Dma,
    PciConfig,
    Spawner,
    Timer,
}

impl Capability {
    /// The `hadron_kernel::driver_api::capability::*` type name.
    pub fn type_ident(&self) -> Ident {
        match self {
            Self::Irq => Ident::new("IrqCapability", Span::call_site()),
            Self::Mmio => Ident::new("MmioCapability", Span::call_site()),
            Self::Dma => Ident::new("DmaCapability", Span::call_site()),
            Self::PciConfig => Ident::new("PciConfigCapability", Span::call_site()),
            Self::Spawner => Ident::new("TaskSpawner", Span::call_site()),
            Self::Timer => Ident::new("TimerCapability", Span::call_site()),
        }
    }

    /// The field name used in the generated context struct.
    pub fn field_ident(&self) -> Ident {
        match self {
            Self::Irq => Ident::new("irq", Span::call_site()),
            Self::Mmio => Ident::new("mmio", Span::call_site()),
            Self::Dma => Ident::new("dma", Span::call_site()),
            Self::PciConfig => Ident::new("pci_config", Span::call_site()),
            Self::Spawner => Ident::new("spawner", Span::call_site()),
            Self::Timer => Ident::new("timer", Span::call_site()),
        }
    }

    /// The corresponding field name on `PciProbeContext` / `PlatformProbeContext`.
    pub fn probe_context_field(&self) -> Ident {
        // Field names match between the context structs and our field names.
        self.field_ident()
    }

    /// The `CapabilityFlags` constant name.
    pub fn flag_ident(&self) -> Ident {
        match self {
            Self::Irq => Ident::new("IRQ", Span::call_site()),
            Self::Mmio => Ident::new("MMIO", Span::call_site()),
            Self::Dma => Ident::new("DMA", Span::call_site()),
            Self::PciConfig => Ident::new("PCI_CONFIG", Span::call_site()),
            Self::Spawner => Ident::new("TASK_SPAWNER", Span::call_site()),
            Self::Timer => Ident::new("TIMER", Span::call_site()),
        }
    }
}

/// Parsed `#[hadron_driver(...)]` attribute.
pub struct DriverDef {
    /// Driver name string (e.g., "ahci").
    pub name: LitStr,
    /// PCI or platform.
    pub kind: DriverKind,
    /// Declared capabilities.
    pub capabilities: Vec<Capability>,
    /// PCI ID table expression (required for `kind = pci`).
    pub pci_ids: Option<Expr>,
    /// Compatible string (required for `kind = platform`).
    pub compatible: Option<LitStr>,
}

impl Parse for DriverDef {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut name: Option<LitStr> = None;
        let mut kind: Option<DriverKind> = None;
        let mut capabilities: Option<Vec<Capability>> = None;
        let mut pci_ids: Option<Expr> = None;
        let mut compatible: Option<LitStr> = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;

            match key.to_string().as_str() {
                "name" => {
                    name = Some(input.parse()?);
                }
                "kind" => {
                    let value: Ident = input.parse()?;
                    kind = Some(match value.to_string().as_str() {
                        "pci" => DriverKind::Pci,
                        "platform" => DriverKind::Platform,
                        _ => {
                            return Err(syn::Error::new(
                                value.span(),
                                "expected `pci` or `platform`",
                            ));
                        }
                    });
                }
                "capabilities" => {
                    let content;
                    syn::bracketed!(content in input);
                    let mut caps = Vec::new();
                    while !content.is_empty() {
                        let cap: Ident = content.parse()?;
                        caps.push(match cap.to_string().as_str() {
                            "Irq" => Capability::Irq,
                            "Mmio" => Capability::Mmio,
                            "Dma" => Capability::Dma,
                            "PciConfig" => Capability::PciConfig,
                            "Spawner" => Capability::Spawner,
                            "Timer" => Capability::Timer,
                            _ => {
                                return Err(syn::Error::new(
                                    cap.span(),
                                    "unknown capability; expected one of: \
                                     Irq, Mmio, Dma, PciConfig, Spawner, Timer",
                                ));
                            }
                        });
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                    }
                    capabilities = Some(caps);
                }
                "pci_ids" => {
                    pci_ids = Some(input.parse()?);
                }
                "compatible" => {
                    compatible = Some(input.parse()?);
                }
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown attribute `{}`; expected one of: \
                             name, kind, capabilities, pci_ids, compatible",
                            key
                        ),
                    ));
                }
            }

            // Consume trailing comma if present.
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        let name = name.ok_or_else(|| {
            syn::Error::new(Span::call_site(), "missing required `name` attribute")
        })?;
        let kind = kind.ok_or_else(|| {
            syn::Error::new(Span::call_site(), "missing required `kind` attribute")
        })?;
        let capabilities = capabilities.ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "missing required `capabilities` attribute",
            )
        })?;

        // Validate kind-specific required fields.
        if kind == DriverKind::Pci && pci_ids.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "`pci_ids` is required for `kind = pci`",
            ));
        }
        if kind == DriverKind::Platform && compatible.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "`compatible` is required for `kind = platform`",
            ));
        }

        Ok(Self {
            name,
            kind,
            capabilities,
            pci_ids,
            compatible,
        })
    }
}
