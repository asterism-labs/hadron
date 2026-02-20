//! AST types for the Kconfig DSL.
//!
//! These types represent the parsed structure of Kconfig files before
//! conversion to the build model's [`ConfigOptionDef`].

use crate::model::Binding;

/// A parsed Kconfig file (may source other files).
#[derive(Debug)]
pub struct KconfigFile {
    pub items: Vec<KconfigItem>,
}

/// A top-level item in a Kconfig file.
#[derive(Debug)]
pub enum KconfigItem {
    /// `config NAME` block with type, default, constraints, and bindings.
    Config(ConfigBlock),
    /// `menu "title"` ... `endmenu` grouping.
    Menu(MenuBlock),
    /// `source "path"` directive.
    Source(String),
}

/// A `config NAME` block.
#[derive(Debug)]
pub struct ConfigBlock {
    pub name: String,
    pub ty: Option<TypeDecl>,
    pub prompt: Option<String>,
    pub default: Option<DefaultValue>,
    pub depends_on: Option<DependsExpr>,
    pub selects: Vec<String>,
    pub range: Option<(u64, u64)>,
    pub bindings: Vec<Binding>,
    pub help: Option<String>,
}

/// Type declaration with optional inline prompt.
#[derive(Debug)]
pub struct TypeDecl {
    pub kind: TypeKind,
    /// For `choice`, the list of variant names.
    pub variants: Vec<String>,
    /// Inline prompt (e.g. `bool "Enable SMP"`).
    pub prompt: Option<String>,
}

/// Config option type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Bool,
    U32,
    U64,
    Str,
    Choice,
}

/// A default value in Kconfig syntax.
#[derive(Debug)]
pub enum DefaultValue {
    /// `y` or `n`.
    Bool(bool),
    /// Decimal or hex integer.
    Integer(u64),
    /// Quoted string.
    Str(String),
}

/// Boolean dependency expression.
#[derive(Debug)]
pub enum DependsExpr {
    /// A single config symbol name.
    Symbol(String),
    /// Logical AND.
    And(Box<DependsExpr>, Box<DependsExpr>),
    /// Logical OR.
    Or(Box<DependsExpr>, Box<DependsExpr>),
    /// Logical NOT.
    Not(Box<DependsExpr>),
}

impl DependsExpr {
    /// Flatten the expression into a list of symbol names that must all be true.
    ///
    /// Only valid for simple AND-chains of symbols. OR/NOT expressions are
    /// flattened conservatively (all referenced symbols are included).
    pub fn flatten_symbols(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_symbols(&mut out);
        out
    }

    fn collect_symbols(&self, out: &mut Vec<String>) {
        match self {
            DependsExpr::Symbol(s) => out.push(s.clone()),
            DependsExpr::And(a, b) | DependsExpr::Or(a, b) => {
                a.collect_symbols(out);
                b.collect_symbols(out);
            }
            DependsExpr::Not(inner) => inner.collect_symbols(out),
        }
    }
}

/// A `menu "title"` ... `endmenu` block.
#[derive(Debug)]
pub struct MenuBlock {
    pub title: String,
    pub items: Vec<KconfigItem>,
}
