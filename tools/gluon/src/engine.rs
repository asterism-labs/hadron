//! Rhai scripting engine for build configuration.
//!
//! Sets up a Rhai engine with builder types and registration functions,
//! evaluates `gluon.rhai`, and produces a [`BuildModel`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rhai::{Dynamic, Engine, Map};

use crate::model::{
    BuildModel, ConfigOptionDef, ConfigType, ConfigValue,
    CrateDef, CrateType, DepDef, DepSource, ExternalDepDef, GitRef,
    GroupDef, PipelineStep, ProfileDef,
    ProjectDef, RuleDef, RuleHandler, TargetDef,
};

/// Shared model state passed to all builder types.
type SharedModel = Arc<Mutex<BuildModel>>;

/// Register a builder method on the Rhai engine.
///
/// Eliminates the repeated `builder.model.lock().unwrap()` + `builder.clone()`
/// boilerplate in builder method registrations. The macro locks the shared model
/// into `$model`, executes `$body`, and returns `builder.clone()`.
///
/// # Variants
///
/// - No extra arguments: `builder_method!(engine, "name", Ty, |b, model| { ... })`
/// - With arguments: `builder_method!(engine, "name", Ty, |b, model, arg: Type| { ... })`
macro_rules! builder_method {
    // No extra arguments beyond the builder itself.
    ($engine:expr, $name:expr, $builder_ty:ty,
     |$builder:ident, $model:ident| $body:block) => {
        $engine.register_fn(
            $name,
            |$builder: &mut $builder_ty| -> $builder_ty {
                #[allow(unused_mut)]
                let mut $model = $builder.model.lock().unwrap();
                $body
                $builder.clone()
            },
        );
    };
    // One or more extra arguments.
    ($engine:expr, $name:expr, $builder_ty:ty,
     |$builder:ident, $model:ident, $($arg:ident : $arg_ty:ty),+| $body:block) => {
        $engine.register_fn(
            $name,
            |$builder: &mut $builder_ty, $($arg: $arg_ty),+| -> $builder_ty {
                #[allow(unused_mut)]
                let mut $model = $builder.model.lock().unwrap();
                $body
                $builder.clone()
            },
        );
    };
}

/// Evaluate `gluon.rhai` from the project root and return the populated model.
pub fn evaluate_script(root: &Path) -> Result<BuildModel> {
    let model = Arc::new(Mutex::new(BuildModel::default()));
    let mut engine = Engine::new();

    engine.set_max_expr_depths(64, 64);
    engine.set_optimization_level(rhai::OptimizationLevel::Full);

    // Store root path for include() and path resolution.
    let root_path = root.to_path_buf();

    let mut scope = rhai::Scope::new();
    scope.push_constant("LIB", 10_i64);
    scope.push_constant("BIN", 11_i64);
    scope.push_constant("PROC_MACRO", 12_i64);

    // Register all API functions.
    register_project_api(&mut engine, model.clone());
    register_target_api(&mut engine, model.clone());
    register_config_api(&mut engine, model.clone());
    register_profile_api(&mut engine, model.clone());
    register_group_api(&mut engine, model.clone());
    register_rule_api(&mut engine, model.clone());
    register_pipeline_api(&mut engine, model.clone());
    register_qemu_api(&mut engine, model.clone());
    register_bootloader_api(&mut engine, model.clone());
    register_image_api(&mut engine, model.clone());
    register_tests_api(&mut engine, model.clone());
    register_benchmarks_api(&mut engine, model.clone());
    register_dependency_api(&mut engine, model.clone());
    register_kconfig_api(&mut engine, model.clone(), &root_path);
    register_helpers(&mut engine, &root_path);

    // Set up include() mechanism with circular-include detection.
    let visited_includes = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
    let script_path = root.join("gluon.rhai");
    if let Ok(canonical) = std::fs::canonicalize(&script_path) {
        visited_includes.lock().unwrap().insert(canonical);
    }
    register_include_api(&mut engine, &root_path, visited_includes);

    // Evaluate gluon.rhai with the scope containing constants.
    crate::verbose::vprintln!("  compiling script: {}", script_path.display());
    let ast = engine
        .compile_file(script_path.clone().into())
        .map_err(|e| anyhow::anyhow!("error compiling {}: {e}", script_path.display()))?;
    crate::verbose::vprintln!("  evaluating script...");
    engine
        .run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| anyhow::anyhow!("error evaluating {}: {e}", script_path.display()))?;

    // Drop the engine, AST, and scope to release all Arc references held by closures.
    drop(ast);
    drop(engine);
    drop(scope);

    let model = Arc::try_unwrap(model)
        .map_err(|_| anyhow::anyhow!("build model still referenced after script evaluation"))?
        .into_inner()
        .map_err(|e| anyhow::anyhow!("poisoned mutex: {e}"))?;

    Ok(model)
}

fn crate_type_from_i64(val: i64) -> CrateType {
    match val {
        11 => CrateType::Bin,
        12 => CrateType::ProcMacro,
        _ => CrateType::Lib,
    }
}

// ---------------------------------------------------------------------------
// project()
// ---------------------------------------------------------------------------

fn register_project_api(engine: &mut Engine, model: SharedModel) {
    engine.register_fn("project", move |name: &str, version: &str| {
        let mut m = model.lock().unwrap();
        m.project = ProjectDef {
            name: name.into(),
            version: version.into(),
        };
    });
}

// ---------------------------------------------------------------------------
// target() -> TargetBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TargetBuilder {
    #[allow(dead_code)] // returned by target() for potential future chaining
    model: SharedModel,
    #[allow(dead_code)] // returned by target() for potential future chaining
    name: String,
}

fn register_target_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("target", move |name: &str, spec: &str| -> TargetBuilder {
        let mut model = m.lock().unwrap();
        model.targets.insert(
            name.into(),
            TargetDef {
                name: name.into(),
                spec: spec.into(),
            },
        );
        TargetBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });
}

// ---------------------------------------------------------------------------
// config_bool(), config_u32(), config_u64(), config_str() -> ConfigBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ConfigBuilder {
    model: SharedModel,
    name: String,
}

#[derive(Debug, Clone)]
struct ConfigGroupBuilder {
    model: SharedModel,
    group_name: String,
}

fn register_config_api(engine: &mut Engine, model: SharedModel) {
    // config_bool(name, default)
    let m = model.clone();
    engine.register_fn("config_bool", move |name: &str, default: bool| -> ConfigBuilder {
        let mut model = m.lock().unwrap();
        model.config_options.insert(
            name.into(),
            ConfigOptionDef {
                name: name.into(),
                ty: ConfigType::Bool,
                default: ConfigValue::Bool(default),
                help: None,
                depends_on: Vec::new(),
                selects: Vec::new(),
                range: None,
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );
        ConfigBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    // config_u32(name, default)
    let m = model.clone();
    engine.register_fn("config_u32", move |name: &str, default: i64| -> ConfigBuilder {
        let mut model = m.lock().unwrap();
        model.config_options.insert(
            name.into(),
            ConfigOptionDef {
                name: name.into(),
                ty: ConfigType::U32,
                default: ConfigValue::U32(default as u32),
                help: None,
                depends_on: Vec::new(),
                selects: Vec::new(),
                range: None,
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );
        ConfigBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    // config_u64(name, default) - accepts string for hex values
    let m = model.clone();
    engine.register_fn("config_u64", move |name: &str, default: &str| -> ConfigBuilder {
        let parsed = parse_integer_str(default).unwrap_or(0);
        let mut model = m.lock().unwrap();
        model.config_options.insert(
            name.into(),
            ConfigOptionDef {
                name: name.into(),
                ty: ConfigType::U64,
                default: ConfigValue::U64(parsed),
                help: None,
                depends_on: Vec::new(),
                selects: Vec::new(),
                range: None,
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );
        ConfigBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    // config_u64(name, default) - accepts integer
    let m = model.clone();
    engine.register_fn("config_u64", move |name: &str, default: i64| -> ConfigBuilder {
        let mut model = m.lock().unwrap();
        model.config_options.insert(
            name.into(),
            ConfigOptionDef {
                name: name.into(),
                ty: ConfigType::U64,
                default: ConfigValue::U64(default as u64),
                help: None,
                depends_on: Vec::new(),
                selects: Vec::new(),
                range: None,
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );
        ConfigBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    // config_str(name, default)
    let m = model.clone();
    engine.register_fn("config_str", move |name: &str, default: &str| -> ConfigBuilder {
        let mut model = m.lock().unwrap();
        model.config_options.insert(
            name.into(),
            ConfigOptionDef {
                name: name.into(),
                ty: ConfigType::Str,
                default: ConfigValue::Str(default.into()),
                help: None,
                depends_on: Vec::new(),
                selects: Vec::new(),
                range: None,
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );
        ConfigBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    // config_choice(name, default, variants) -> ConfigBuilder
    let m = model.clone();
    engine.register_fn(
        "config_choice",
        move |name: &str, default: &str, variants: rhai::Array| -> ConfigBuilder {
            let choices: Vec<String> = variants
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
            let mut model = m.lock().unwrap();
            model.config_options.insert(
                name.into(),
                ConfigOptionDef {
                    name: name.into(),
                    ty: ConfigType::Choice,
                    default: ConfigValue::Choice(default.into()),
                    help: None,
                    depends_on: Vec::new(),
                    selects: Vec::new(),
                    range: None,
                    choices: Some(choices),
                    menu: None,
                    bindings: Vec::new(),
                },
            );
            ConfigBuilder {
                model: m.clone(),
                name: name.into(),
            }
        },
    );

    // config_list(name, default_array) -> ConfigBuilder
    let m = model.clone();
    engine.register_fn(
        "config_list",
        move |name: &str, defaults: rhai::Array| -> ConfigBuilder {
            let items: Vec<String> = defaults
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
            let mut model = m.lock().unwrap();
            model.config_options.insert(
                name.into(),
                ConfigOptionDef {
                    name: name.into(),
                    ty: ConfigType::List,
                    default: ConfigValue::List(items),
                    help: None,
                    depends_on: Vec::new(),
                    selects: Vec::new(),
                    range: None,
                    choices: None,
                    menu: None,
                    bindings: Vec::new(),
                },
            );
            ConfigBuilder {
                model: m.clone(),
                name: name.into(),
            }
        },
    );

    // config_group(name) -> ConfigGroupBuilder
    let m = model.clone();
    engine.register_fn(
        "config_group",
        move |name: &str| -> ConfigGroupBuilder {
            let mut model = m.lock().unwrap();
            model.config_options.insert(
                name.into(),
                ConfigOptionDef {
                    name: name.into(),
                    ty: ConfigType::Group,
                    default: ConfigValue::Bool(false), // sentinel, not used
                    help: None,
                    depends_on: Vec::new(),
                    selects: Vec::new(),
                    range: None,
                    choices: None,
                    menu: None,
                    bindings: Vec::new(),
                },
            );
            ConfigGroupBuilder {
                model: m.clone(),
                group_name: name.into(),
            }
        },
    );

    // ConfigGroupBuilder methods
    builder_method!(engine, "field", ConfigGroupBuilder,
        |builder, model, name: &str, value: Dynamic| {
            let dotted_key = format!("{}.{name}", builder.group_name);
            let config_val = dynamic_to_config_value(&value);
            let ty = match &config_val {
                ConfigValue::Bool(_) => ConfigType::Bool,
                ConfigValue::U32(_) => ConfigType::U32,
                ConfigValue::U64(_) => ConfigType::U64,
                ConfigValue::Str(_) => ConfigType::Str,
                ConfigValue::Choice(_) => ConfigType::Choice,
                ConfigValue::List(_) => ConfigType::List,
            };
            // Inherit menu from group marker.
            let menu = model
                .config_options
                .get(&builder.group_name)
                .and_then(|o| o.menu.clone());
            model.config_options.insert(
                dotted_key.clone(),
                ConfigOptionDef {
                    name: dotted_key,
                    ty,
                    default: config_val,
                    help: None,
                    depends_on: Vec::new(),
                    selects: Vec::new(),
                    range: None,
                    choices: None,
                    menu,
                    bindings: Vec::new(),
                },
            );
        }
    );

    builder_method!(engine, "help", ConfigGroupBuilder,
        |builder, model, help: &str| {
            if let Some(opt) = model.config_options.get_mut(&builder.group_name) {
                opt.help = Some(help.into());
            }
        }
    );

    builder_method!(engine, "menu", ConfigGroupBuilder,
        |builder, model, menu: &str| {
            if let Some(opt) = model.config_options.get_mut(&builder.group_name) {
                opt.menu = Some(menu.into());
            }
            // Track menu category ordering (first-appearance).
            if !model.menu_order.iter().any(|m| m == menu) {
                model.menu_order.push(menu.into());
            }
        }
    );

    // ConfigBuilder methods
    builder_method!(engine, "depends_on", ConfigBuilder,
        |builder, model, deps: rhai::Array| {
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.depends_on = deps
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );

    builder_method!(engine, "selects", ConfigBuilder,
        |builder, model, sels: rhai::Array| {
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.selects = sels
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );

    builder_method!(engine, "range", ConfigBuilder,
        |builder, model, min: i64, max: i64| {
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.range = Some((min as u64, max as u64));
            }
        }
    );

    builder_method!(engine, "choices", ConfigBuilder,
        |builder, model, choices: rhai::Array| {
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.choices = Some(
                    choices
                        .into_iter()
                        .filter_map(|v| v.into_string().ok())
                        .collect(),
                );
            }
        }
    );

    builder_method!(engine, "help", ConfigBuilder,
        |builder, model, help: &str| {
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.help = Some(help.into());
            }
        }
    );

    builder_method!(engine, "menu", ConfigBuilder,
        |builder, model, menu: &str| {
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.menu = Some(menu.into());
            }
            // Track menu category ordering (first-appearance).
            if !model.menu_order.iter().any(|m| m == menu) {
                model.menu_order.push(menu.into());
            }
        }
    );
}

// ---------------------------------------------------------------------------
// profile() -> ProfileBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ProfileBuilder {
    model: SharedModel,
    name: String,
}

fn register_profile_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("profile", move |name: &str| -> ProfileBuilder {
        let mut model = m.lock().unwrap();
        model.profiles.entry(name.into()).or_insert_with(|| ProfileDef {
            name: name.into(),
            ..Default::default()
        });
        ProfileBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    builder_method!(engine, "inherits", ProfileBuilder,
        |builder, model, parent: &str| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.inherits = Some(parent.into());
            }
        }
    );

    builder_method!(engine, "target", ProfileBuilder,
        |builder, model, target: &str| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.target = Some(target.into());
            }
        }
    );

    builder_method!(engine, "opt_level", ProfileBuilder,
        |builder, model, level: i64| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.opt_level = Some(level as u32);
            }
        }
    );

    builder_method!(engine, "debug_info", ProfileBuilder,
        |builder, model, enabled: bool| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.debug_info = Some(enabled);
            }
        }
    );

    builder_method!(engine, "lto", ProfileBuilder,
        |builder, model, lto: &str| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.lto = Some(lto.into());
            }
        }
    );

    builder_method!(engine, "boot_binary", ProfileBuilder,
        |builder, model, bin: &str| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.boot_binary = Some(bin.into());
            }
        }
    );

    builder_method!(engine, "preset", ProfileBuilder,
        |builder, model, preset_name: &str| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.preset = Some(preset_name.into());
            }
        }
    );

    builder_method!(engine, "config", ProfileBuilder,
        |builder, model, overrides: Map| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                for (key, val) in overrides {
                    let config_val = dynamic_to_config_value(&val);
                    p.config.insert(key.to_string(), config_val);
                }
            }
        }
    );

    builder_method!(engine, "qemu_memory", ProfileBuilder,
        |builder, model, mem: i64| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.qemu_memory = Some(mem as u32);
            }
        }
    );

    builder_method!(engine, "qemu_cores", ProfileBuilder,
        |builder, model, cores: i64| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.qemu_cores = Some(cores as u32);
            }
        }
    );

    builder_method!(engine, "qemu_extra_args", ProfileBuilder,
        |builder, model, args: rhai::Array| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.qemu_extra_args = Some(
                    args.into_iter()
                        .filter_map(|v| v.into_string().ok())
                        .collect(),
                );
            }
        }
    );

    builder_method!(engine, "test_timeout", ProfileBuilder,
        |builder, model, timeout: i64| {
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.test_timeout = Some(timeout as u32);
            }
        }
    );
}

// ---------------------------------------------------------------------------
// group() -> GroupBuilder,  GroupBuilder.add() -> CrateBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct GroupBuilder {
    model: SharedModel,
    name: String,
}

#[derive(Debug, Clone)]
struct CrateBuilder {
    model: SharedModel,
    name: String,
}

fn register_group_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("group", move |name: &str| -> GroupBuilder {
        let mut model = m.lock().unwrap();
        model.groups.entry(name.into()).or_insert_with(|| GroupDef {
            name: name.into(),
            ..Default::default()
        });
        GroupBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    builder_method!(engine, "target", GroupBuilder,
        |builder, model, target: &str| {
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.target = target.into();
            }
        }
    );

    builder_method!(engine, "config", GroupBuilder,
        |builder, model, has_config: bool| {
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.config = has_config;
            }
        }
    );

    builder_method!(engine, "edition", GroupBuilder,
        |builder, model, ed: &str| {
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.default_edition = ed.into();
            }
        }
    );

    builder_method!(engine, "project", GroupBuilder,
        |builder, model, is_proj: bool| {
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.is_project = is_proj;
            }
        }
    );

    // group.add(name, path) -> CrateBuilder (returns a different type, not macro-eligible)
    engine.register_fn(
        "add",
        |builder: &mut GroupBuilder, name: &str, path: &str| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            let group = model.groups.get_mut(&builder.name).unwrap();
            let edition = group.default_edition.clone();
            let target = group.target.clone();
            let is_project = group.is_project;
            group.crates.push(name.into());

            model.crates.insert(
                name.into(),
                CrateDef {
                    name: name.into(),
                    path: path.into(),
                    edition,
                    crate_type: CrateType::Lib,
                    target,
                    deps: std::collections::BTreeMap::new(),
                    dev_deps: std::collections::BTreeMap::new(),
                    features: Vec::new(),
                    root: None,
                    linker_script: None,
                    group: Some(builder.name.clone()),
                    is_project_crate: is_project,
                    cfg_flags: Vec::new(),
                    requires_config: Vec::new(),
                },
            );
            CrateBuilder {
                model: builder.model.clone(),
                name: name.into(),
            }
        },
    );

    // CrateBuilder methods.
    builder_method!(engine, "deps", CrateBuilder,
        |builder, model, deps: Map| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                for (extern_name, val) in deps {
                    let dep = parse_dep_value(&extern_name, &val);
                    krate.deps.insert(extern_name.to_string(), dep);
                }
            }
        }
    );

    builder_method!(engine, "dev_deps", CrateBuilder,
        |builder, model, deps: Map| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                for (extern_name, val) in deps {
                    let dep = parse_dep_value(&extern_name, &val);
                    krate.dev_deps.insert(extern_name.to_string(), dep);
                }
            }
        }
    );

    builder_method!(engine, "features", CrateBuilder,
        |builder, model, feats: rhai::Array| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.features = feats
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );

    builder_method!(engine, "crate_type", CrateBuilder,
        |builder, model, ty: i64| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.crate_type = crate_type_from_i64(ty);
            }
        }
    );

    builder_method!(engine, "root", CrateBuilder,
        |builder, model, root: &str| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.root = Some(root.into());
            }
        }
    );

    builder_method!(engine, "edition", CrateBuilder,
        |builder, model, ed: &str| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.edition = ed.into();
            }
        }
    );

    builder_method!(engine, "linker_script", CrateBuilder,
        |builder, model, script: &str| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.linker_script = Some(script.into());
            }
        }
    );

    builder_method!(engine, "requires_config", CrateBuilder,
        |builder, model, config_name: &str| {
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.requires_config.push(config_name.into());
            }
        }
    );
}

// ---------------------------------------------------------------------------
// rule() -> RuleBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RuleBuilder {
    model: SharedModel,
    name: String,
}

fn register_rule_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("rule", move |name: &str| -> RuleBuilder {
        let mut model = m.lock().unwrap();
        model.rules.entry(name.into()).or_insert_with(|| RuleDef {
            name: name.into(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            depends_on: Vec::new(),
            handler: RuleHandler::Builtin(name.into()),
        });
        RuleBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    builder_method!(engine, "inputs", RuleBuilder,
        |builder, model, inputs: rhai::Array| {
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.inputs = inputs
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );

    builder_method!(engine, "output", RuleBuilder,
        |builder, model, output: &str| {
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.outputs = vec![output.into()];
            }
        }
    );

    builder_method!(engine, "outputs", RuleBuilder,
        |builder, model, outputs: rhai::Array| {
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.outputs = outputs
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );

    builder_method!(engine, "depends_on", RuleBuilder,
        |builder, model, deps: rhai::Array| {
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.depends_on = deps
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );

    builder_method!(engine, "handler", RuleBuilder,
        |builder, model, handler: &str| {
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.handler = RuleHandler::Builtin(handler.into());
            }
        }
    );
}

// ---------------------------------------------------------------------------
// pipeline() -> PipelineBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct PipelineBuilder {
    model: SharedModel,
}

fn register_pipeline_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("pipeline", move || -> PipelineBuilder {
        PipelineBuilder { model: m.clone() }
    });

    builder_method!(engine, "stage", PipelineBuilder,
        |builder, model, name: &str, groups: rhai::Array| {
            model.pipeline.steps.push(PipelineStep::Stage {
                name: name.into(),
                groups: groups
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect(),
            });
        }
    );

    builder_method!(engine, "barrier", PipelineBuilder,
        |builder, model, name: &str| {
            model.pipeline.steps.push(PipelineStep::Barrier(name.into()));
        }
    );

    builder_method!(engine, "rule", PipelineBuilder,
        |builder, model, name: &str| {
            model.pipeline.steps.push(PipelineStep::Rule(name.into()));
        }
    );
}

// ---------------------------------------------------------------------------
// qemu()
// ---------------------------------------------------------------------------

fn register_qemu_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("qemu", move |machine: &str, memory: i64| -> QemuBuilder {
        let mut model = m.lock().unwrap();
        model.qemu.machine = machine.into();
        model.qemu.memory = memory as u32;
        QemuBuilder { model: m.clone() }
    });

    builder_method!(engine, "extra_args", QemuBuilder,
        |builder, model, args: rhai::Array| {
            model.qemu.extra_args = args
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
        }
    );

    builder_method!(engine, "test_success_code", QemuBuilder,
        |builder, model, code: i64| {
            model.qemu.test.success_exit_code = code as u32;
        }
    );

    builder_method!(engine, "test_timeout", QemuBuilder,
        |builder, model, timeout: i64| {
            model.qemu.test.timeout = timeout as u32;
        }
    );

    builder_method!(engine, "test_extra_args", QemuBuilder,
        |builder, model, args: rhai::Array| {
            model.qemu.test.extra_args = args
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
        }
    );
}

#[derive(Debug, Clone)]
struct QemuBuilder {
    model: SharedModel,
}

// ---------------------------------------------------------------------------
// bootloader()
// ---------------------------------------------------------------------------

fn register_bootloader_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("bootloader", move |kind: &str| -> BootloaderBuilder {
        let mut model = m.lock().unwrap();
        model.bootloader.kind = kind.into();
        BootloaderBuilder { model: m.clone() }
    });

    builder_method!(engine, "config_file", BootloaderBuilder,
        |builder, model, file: &str| {
            model.bootloader.config_file = Some(file.into());
        }
    );
}

#[derive(Debug, Clone)]
struct BootloaderBuilder {
    model: SharedModel,
}

// ---------------------------------------------------------------------------
// image()
// ---------------------------------------------------------------------------

fn register_image_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("image", move || -> ImageBuilder {
        ImageBuilder { model: m.clone() }
    });

    builder_method!(engine, "extra_files", ImageBuilder,
        |builder, model, files: Map| {
            for (key, val) in files {
                if let Ok(v) = val.into_string() {
                    model.image.extra_files.insert(key.to_string(), v);
                }
            }
        }
    );
}

#[derive(Debug, Clone)]
struct ImageBuilder {
    model: SharedModel,
}

// ---------------------------------------------------------------------------
// tests()
// ---------------------------------------------------------------------------

fn register_tests_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("tests", move || -> TestsBuilder {
        TestsBuilder { model: m.clone() }
    });

    builder_method!(engine, "host_testable", TestsBuilder,
        |builder, model, crates: rhai::Array| {
            model.tests.host_testable = crates
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
        }
    );

    builder_method!(engine, "kernel_tests_dir", TestsBuilder,
        |builder, model, dir: &str| {
            model.tests.kernel_tests_dir = Some(dir.into());
        }
    );

    builder_method!(engine, "kernel_tests_crate", TestsBuilder,
        |builder, model, name: &str| {
            model.tests.kernel_tests_crate = Some(name.into());
        }
    );

    builder_method!(engine, "kernel_tests_linker_script", TestsBuilder,
        |builder, model, path: &str| {
            model.tests.kernel_tests_linker_script = Some(path.into());
        }
    );
}

#[derive(Debug, Clone)]
struct TestsBuilder {
    model: SharedModel,
}

// ---------------------------------------------------------------------------
// benchmarks()
// ---------------------------------------------------------------------------

fn register_benchmarks_api(engine: &mut Engine, model: SharedModel) {
    let m = model.clone();
    engine.register_fn("benchmarks", move || -> BenchmarksBuilder {
        BenchmarksBuilder { model: m.clone() }
    });

    builder_method!(engine, "benches_dir", BenchmarksBuilder,
        |builder, model, dir: &str| {
            model.benchmarks.benches_dir = Some(dir.into());
        }
    );

    builder_method!(engine, "benches_crate", BenchmarksBuilder,
        |builder, model, name: &str| {
            model.benchmarks.benches_crate = Some(name.into());
        }
    );

    builder_method!(engine, "benches_linker_script", BenchmarksBuilder,
        |builder, model, path: &str| {
            model.benchmarks.benches_linker_script = Some(path.into());
        }
    );
}

#[derive(Debug, Clone)]
struct BenchmarksBuilder {
    model: SharedModel,
}

// ---------------------------------------------------------------------------
// dependency() -> DependencyBuilder
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct DependencyBuilder {
    model: SharedModel,
    name: String,
}

fn register_dependency_api(engine: &mut Engine, model: SharedModel) {
    // dependency(name) -> DependencyBuilder
    let m = model.clone();
    engine.register_fn("dependency", move |name: &str| -> DependencyBuilder {
        let mut model = m.lock().unwrap();
        model.dependencies.entry(name.into()).or_insert_with(|| ExternalDepDef {
            name: name.into(),
            source: DepSource::CratesIo { version: String::new() },
            features: Vec::new(),
            default_features: true,
            cfg_flags: Vec::new(),
        });
        DependencyBuilder {
            model: m.clone(),
            name: name.into(),
        }
    });

    // .version(str) — sets crates.io source
    builder_method!(engine, "version", DependencyBuilder,
        |builder, model, version: &str| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                dep.source = DepSource::CratesIo { version: version.into() };
            }
        }
    );

    // .git(url) — sets git source with default ref
    builder_method!(engine, "git", DependencyBuilder,
        |builder, model, url: &str| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                dep.source = DepSource::Git {
                    url: url.into(),
                    reference: GitRef::Default,
                };
            }
        }
    );

    // .path(path) — sets local path source
    builder_method!(engine, "path", DependencyBuilder,
        |builder, model, path: &str| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                dep.source = DepSource::Path { path: path.into() };
            }
        }
    );

    // .rev(str) — refine git reference to a specific commit
    builder_method!(engine, "rev", DependencyBuilder,
        |builder, model, rev: &str| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                if let DepSource::Git { ref mut reference, .. } = dep.source {
                    *reference = GitRef::Rev(rev.into());
                }
            }
        }
    );

    // .tag(str) — refine git reference to a tag
    builder_method!(engine, "tag", DependencyBuilder,
        |builder, model, tag: &str| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                if let DepSource::Git { ref mut reference, .. } = dep.source {
                    *reference = GitRef::Tag(tag.into());
                }
            }
        }
    );

    // .branch(str) — refine git reference to a branch
    builder_method!(engine, "branch", DependencyBuilder,
        |builder, model, branch: &str| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                if let DepSource::Git { ref mut reference, .. } = dep.source {
                    *reference = GitRef::Branch(branch.into());
                }
            }
        }
    );

    // .features([...]) — add extra features
    builder_method!(engine, "features", DependencyBuilder,
        |builder, model, feats: rhai::Array| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                dep.features = feats
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );

    // .no_default_features() — disable default features
    builder_method!(engine, "no_default_features", DependencyBuilder,
        |builder, model| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                dep.default_features = false;
            }
        }
    );

    // .cfg_flags([...]) — extra --cfg flags for compilation
    builder_method!(engine, "cfg_flags", DependencyBuilder,
        |builder, model, flags: rhai::Array| {
            if let Some(dep) = model.dependencies.get_mut(&builder.name) {
                dep.cfg_flags = flags
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
        }
    );
}

// ---------------------------------------------------------------------------
// include() custom syntax
// ---------------------------------------------------------------------------

/// Shared set of already-visited include paths (for once-only semantics).
type VisitedIncludes = Arc<Mutex<HashSet<PathBuf>>>;

fn register_include_api(engine: &mut Engine, root: &Path, visited: VisitedIncludes) {
    let root_path = root.to_path_buf();

    engine
        .register_custom_syntax(
            ["include", "$expr$"],
            true, // scope may be changed
            move |context, inputs| {
                // Evaluate the path expression.
                let path_val = context.eval_expression_tree(&inputs[0])?;
                let rel_path: String = path_val
                    .into_string()
                    .map_err(|e| {
                        Box::new(rhai::EvalAltResult::ErrorMismatchDataType(
                            "string".into(),
                            e.into(),
                            rhai::Position::NONE,
                        ))
                    })?;

                // Resolve against project root (root-relative).
                let abs_path = root_path.join(&rel_path);
                let canonical = std::fs::canonicalize(&abs_path).map_err(|e| {
                    Box::new(rhai::EvalAltResult::ErrorSystem(
                        format!("include '{rel_path}'"),
                        Box::new(e),
                    ))
                })?;

                // Once-only: skip if already included.
                {
                    let mut set = visited.lock().unwrap();
                    if set.contains(&canonical) {
                        return Ok(Dynamic::UNIT);
                    }
                    set.insert(canonical.clone());
                }

                // Compile and evaluate the included file with shared scope.
                let ast =
                    context.engine().compile_file(canonical.into()).map_err(|e| {
                        Box::new(rhai::EvalAltResult::ErrorSystem(
                            format!("while including '{rel_path}'"),
                            Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())),
                        ))
                    })?;

                context
                    .engine()
                    .run_ast_with_scope(context.scope_mut(), &ast)
                    .map_err(|e| {
                        Box::new(rhai::EvalAltResult::ErrorSystem(
                            format!("while including '{rel_path}'"),
                            Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())),
                        ))
                    })?;

                Ok(Dynamic::UNIT)
            },
        )
        .expect("failed to register include syntax");
}

// ---------------------------------------------------------------------------
// kconfig() — Load Kconfig DSL files
// ---------------------------------------------------------------------------

fn register_kconfig_api(engine: &mut Engine, model: SharedModel, root: &Path) {
    let m = model;
    let root_path = root.to_path_buf();
    engine.register_fn("kconfig", move |path: &str| {
        let (options, order, presets, files) = crate::kconfig::load_kconfig(&root_path, path)
            .unwrap_or_else(|e| panic!("kconfig error: {e}"));
        let mut model = m.lock().unwrap();
        model.config_options.extend(options);
        for menu in order {
            if !model.menu_order.iter().any(|m| m == &menu) {
                model.menu_order.push(menu);
            }
        }
        model.presets.extend(presets);
        // Track kconfig files for model cache invalidation.
        model.input_files.extend(files);
    });
}

// ---------------------------------------------------------------------------
// Helper functions available in scripts
// ---------------------------------------------------------------------------

fn register_helpers(engine: &mut Engine, root: &Path) {
    let root_for_project = root.to_path_buf();

    // project_root() -> string
    engine.register_fn("project_root", move || -> String {
        root_for_project.to_string_lossy().into_owned()
    });

    // host_triple() -> string
    engine.register_fn("host_triple", || -> String {
        crate::rustc_info::host_triple().to_string()
    });

    // env("VAR") -> string
    engine.register_fn("env", |var: &str| -> String {
        std::env::var(var).unwrap_or_default()
    });
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Parse a hex or decimal integer string.
fn parse_integer_str(s: &str) -> Option<u64> {
    let s = s.replace('_', "");
    if let Some(hex) = s.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

/// Convert a Rhai Dynamic value to a ConfigValue.
fn dynamic_to_config_value(val: &Dynamic) -> ConfigValue {
    if let Some(b) = val.as_bool().ok() {
        ConfigValue::Bool(b)
    } else if let Some(i) = val.as_int().ok() {
        ConfigValue::U32(i as u32)
    } else if let Some(arr) = val.clone().try_cast::<rhai::Array>() {
        let items: Vec<String> = arr
            .into_iter()
            .filter_map(|v| v.into_string().ok())
            .collect();
        ConfigValue::List(items)
    } else if let Some(s) = val.clone().into_string().ok() {
        // Check if it's a hex number string.
        if let Some(v) = parse_integer_str(&s) {
            ConfigValue::U64(v)
        } else {
            ConfigValue::Str(s)
        }
    } else {
        ConfigValue::Bool(false)
    }
}

/// Parse a dependency value from a Rhai map.
///
/// Supports two forms:
/// - Simple: `extern_name: "crate-name"` (string value)
/// - Table: `extern_name: #{ crate: "name", proc_macro: true }` (map value)
fn parse_dep_value(extern_name: &str, val: &Dynamic) -> DepDef {
    if let Ok(crate_name) = val.clone().into_string() {
        DepDef {
            extern_name: extern_name.into(),
            crate_name,
            features: Vec::new(),
            version: None,
        }
    } else if let Some(map) = val.read_lock::<Map>() {
        let crate_name = map
            .get("crate")
            .and_then(|v| v.clone().into_string().ok())
            .unwrap_or_else(|| extern_name.into());
        let features = map
            .get("features")
            .and_then(|v| v.clone().try_cast::<rhai::Array>())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        DepDef {
            extern_name: extern_name.into(),
            crate_name,
            features,
            version: None,
        }
    } else {
        DepDef {
            extern_name: extern_name.into(),
            crate_name: extern_name.into(),
            features: Vec::new(),
            version: None,
        }
    }
}
