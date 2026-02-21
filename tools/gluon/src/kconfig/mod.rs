//! Kconfig DSL parser for declarative configuration.
//!
//! Parses Kconfig files into an AST, then converts to the build model's
//! [`ConfigOptionDef`] types. Follows `source` directives to load
//! distributed config files near the code they affect.

pub mod ast;
pub mod lexer;
pub mod parser;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::{ConfigOptionDef, ConfigType, ConfigValue};

use ast::*;
use parser::Parser;

/// Parse a root Kconfig file and all sourced sub-files.
///
/// Returns the parsed config options, the menu ordering, and all file paths loaded.
pub fn load_kconfig(
    root_path: &Path,
    kconfig_path: &str,
) -> Result<(BTreeMap<String, ConfigOptionDef>, Vec<String>, Vec<PathBuf>), String> {
    crate::verbose::vprintln!("  loading kconfig: {}", kconfig_path);
    let abs_path = root_path.join(kconfig_path);
    let file = parse_file(&abs_path)?;

    let mut options = BTreeMap::new();
    let mut menu_order = Vec::new();
    let mut loaded_files = vec![abs_path];

    process_items(&file.items, root_path, None, &mut options, &mut menu_order, &mut loaded_files)?;
    crate::verbose::vprintln!("  kconfig: {} options from {} files", options.len(), loaded_files.len());

    Ok((options, menu_order, loaded_files))
}

/// Parse a single Kconfig file into an AST.
fn parse_file(path: &Path) -> Result<KconfigFile, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let tokens = lexer::tokenize(&source, path.to_path_buf())?;
    let mut parser = Parser::new(tokens);
    parser.parse()
}

/// Recursively process AST items, resolving sources and converting configs.
fn process_items(
    items: &[KconfigItem],
    root_path: &Path,
    menu_title: Option<&str>,
    options: &mut BTreeMap<String, ConfigOptionDef>,
    menu_order: &mut Vec<String>,
    loaded_files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    // Track menu for ordering
    if let Some(title) = menu_title {
        if !menu_order.iter().any(|m| m == title) {
            menu_order.push(title.to_string());
        }
    }

    for item in items {
        match item {
            KconfigItem::Config(block) => {
                let opt = convert_config_block(block, menu_title)?;
                options.insert(opt.name.clone(), opt);
            }
            KconfigItem::Menu(menu) => {
                process_items(&menu.items, root_path, Some(&menu.title), options, menu_order, loaded_files)?;
            }
            KconfigItem::Source(path) => {
                let abs_path = root_path.join(path);
                let sub_file = parse_file(&abs_path)?;
                loaded_files.push(abs_path);
                process_items(&sub_file.items, root_path, menu_title, options, menu_order, loaded_files)?;
            }
        }
    }

    Ok(())
}

/// Convert a parsed [`ConfigBlock`] into a [`ConfigOptionDef`].
fn convert_config_block(
    block: &ConfigBlock,
    menu_title: Option<&str>,
) -> Result<ConfigOptionDef, String> {
    let ty_decl = block.ty.as_ref()
        .ok_or_else(|| format!("config '{}' has no type declaration", block.name))?;

    let (config_type, default, choices) = match ty_decl.kind {
        TypeKind::Bool => {
            let default = match &block.default {
                Some(DefaultValue::Bool(v)) => ConfigValue::Bool(*v),
                None => ConfigValue::Bool(false),
                _ => return Err(format!("config '{}': bool expects y/n default", block.name)),
            };
            (ConfigType::Bool, default, None)
        }
        TypeKind::U32 => {
            let default = match &block.default {
                Some(DefaultValue::Integer(v)) => ConfigValue::U32(*v as u32),
                None => ConfigValue::U32(0),
                _ => return Err(format!("config '{}': u32 expects integer default", block.name)),
            };
            (ConfigType::U32, default, None)
        }
        TypeKind::U64 => {
            let default = match &block.default {
                Some(DefaultValue::Integer(v)) => ConfigValue::U64(*v),
                None => ConfigValue::U64(0),
                _ => return Err(format!("config '{}': u64 expects integer default", block.name)),
            };
            (ConfigType::U64, default, None)
        }
        TypeKind::Str => {
            let default = match &block.default {
                Some(DefaultValue::Str(s)) => ConfigValue::Str(s.clone()),
                None => ConfigValue::Str(String::new()),
                _ => return Err(format!("config '{}': str expects string default", block.name)),
            };
            (ConfigType::Str, default, None)
        }
        TypeKind::Choice => {
            let variants = &ty_decl.variants;
            if variants.is_empty() {
                return Err(format!("config '{}': choice requires variant list", block.name));
            }
            let default = match &block.default {
                Some(DefaultValue::Str(s)) => ConfigValue::Choice(s.clone()),
                None => ConfigValue::Choice(variants[0].clone()),
                _ => return Err(format!("config '{}': choice expects string default", block.name)),
            };
            (ConfigType::Choice, default, Some(variants.clone()))
        }
    };

    // Flatten depends expression into symbol list
    let depends_on = block.depends_on.as_ref()
        .map(|expr| expr.flatten_symbols())
        .unwrap_or_default();

    // Use prompt from type decl or explicit prompt
    let help = block.prompt.clone()
        .or_else(|| ty_decl.prompt.clone())
        .or_else(|| block.help.clone());

    Ok(ConfigOptionDef {
        name: block.name.clone(),
        ty: config_type,
        default,
        help,
        depends_on,
        selects: block.selects.clone(),
        range: block.range,
        choices,
        menu: menu_title.map(String::from),
        bindings: block.bindings.clone(),
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Binding;

    #[test]
    fn convert_bool_config() {
        let block = ConfigBlock {
            name: "SMP".to_string(),
            ty: Some(TypeDecl {
                kind: TypeKind::Bool,
                variants: Vec::new(),
                prompt: Some("Enable SMP".to_string()),
            }),
            prompt: None,
            default: Some(DefaultValue::Bool(true)),
            depends_on: Some(DependsExpr::Symbol("ACPI".to_string())),
            selects: vec!["APIC".to_string()],
            range: None,
            bindings: vec![Binding::Cfg, Binding::Build],
            help: None,
        };

        let opt = convert_config_block(&block, Some("SMP")).unwrap();
        assert_eq!(opt.name, "SMP");
        assert_eq!(opt.ty, ConfigType::Bool);
        assert!(matches!(opt.default, ConfigValue::Bool(true)));
        assert_eq!(opt.depends_on, vec!["ACPI"]);
        assert_eq!(opt.selects, vec!["APIC"]);
        assert_eq!(opt.menu, Some("SMP".to_string()));
        assert_eq!(opt.bindings, vec![Binding::Cfg, Binding::Build]);
    }

    #[test]
    fn convert_choice_config() {
        let block = ConfigBlock {
            name: "LOG_LEVEL".to_string(),
            ty: Some(TypeDecl {
                kind: TypeKind::Choice,
                variants: vec!["error".into(), "warn".into(), "info".into(), "debug".into(), "trace".into()],
                prompt: Some("Log level".to_string()),
            }),
            prompt: None,
            default: Some(DefaultValue::Str("debug".to_string())),
            depends_on: None,
            selects: Vec::new(),
            range: None,
            bindings: vec![Binding::CfgCumulative, Binding::Const],
            help: None,
        };

        let opt = convert_config_block(&block, Some("General")).unwrap();
        assert_eq!(opt.ty, ConfigType::Choice);
        assert!(matches!(&opt.default, ConfigValue::Choice(s) if s == "debug"));
        assert_eq!(opt.choices.as_ref().unwrap().len(), 5);
        assert_eq!(opt.bindings, vec![Binding::CfgCumulative, Binding::Const]);
    }

    #[test]
    fn convert_u32_with_range() {
        let block = ConfigBlock {
            name: "MAX_CPUS".to_string(),
            ty: Some(TypeDecl {
                kind: TypeKind::U32,
                variants: Vec::new(),
                prompt: Some("Max CPUs".to_string()),
            }),
            prompt: None,
            default: Some(DefaultValue::Integer(128)),
            depends_on: None,
            selects: Vec::new(),
            range: Some((1, 256)),
            bindings: vec![Binding::Const],
            help: None,
        };

        let opt = convert_config_block(&block, None).unwrap();
        assert_eq!(opt.name, "MAX_CPUS");
        assert_eq!(opt.ty, ConfigType::U32);
        assert!(matches!(opt.default, ConfigValue::U32(128)));
        assert_eq!(opt.range, Some((1, 256)));
        assert_eq!(opt.menu, None);
        assert_eq!(opt.bindings, vec![Binding::Const]);
    }

    #[test]
    fn convert_u64_hex_default() {
        let block = ConfigBlock {
            name: "FRAMEBUFFER_ADDR".to_string(),
            ty: Some(TypeDecl {
                kind: TypeKind::U64,
                variants: Vec::new(),
                prompt: Some("Framebuffer address".to_string()),
            }),
            prompt: None,
            default: Some(DefaultValue::Integer(0xFF0000)),
            depends_on: None,
            selects: Vec::new(),
            range: None,
            bindings: vec![Binding::Build],
            help: None,
        };

        let opt = convert_config_block(&block, Some("Display")).unwrap();
        assert_eq!(opt.name, "FRAMEBUFFER_ADDR");
        assert_eq!(opt.ty, ConfigType::U64);
        assert!(matches!(opt.default, ConfigValue::U64(0xFF0000)));
        assert_eq!(opt.menu, Some("Display".to_string()));
    }

    #[test]
    fn convert_no_type_error() {
        let block = ConfigBlock {
            name: "BROKEN".to_string(),
            ty: None,
            prompt: None,
            default: None,
            depends_on: None,
            selects: Vec::new(),
            range: None,
            bindings: Vec::new(),
            help: None,
        };

        let result = convert_config_block(&block, None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no type"),
            "error message should mention 'no type', got: {err}"
        );
    }
}
