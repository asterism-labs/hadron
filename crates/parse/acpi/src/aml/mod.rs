//! AML (ACPI Machine Language) bytecode parsing.
//!
//! This module provides a single-pass namespace walker that extracts the
//! static topology from DSDT/SSDT AML bytecode: devices, scopes, methods,
//! name objects, thermal zones, processors, and power resources.
//!
//! The [`visitor::AmlVisitor`] trait provides a zero-allocation callback
//! interface. When the `alloc` feature is enabled, [`namespace::NamespaceBuilder`]
//! collects the namespace into a searchable tree.

pub mod parser;
pub mod path;
pub mod value;
pub mod visitor;

#[cfg(feature = "alloc")]
pub mod namespace;

pub use parser::walk_aml;
pub use path::{AmlPath, NameSeg};
pub use value::{AmlError, AmlValue, EisaId, InlineString};
pub use visitor::AmlVisitor;

#[cfg(feature = "alloc")]
pub use namespace::{Namespace, NamespaceBuilder, NamespaceNode, NodeKind};
