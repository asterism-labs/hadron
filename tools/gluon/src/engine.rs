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
    CrateDef, CrateType, DepDef, GroupDef, PipelineStep, ProfileDef,
    ProjectDef, RuleDef, RuleHandler, TargetDef,
};

/// Shared model state passed to all builder types.
type SharedModel = Arc<Mutex<BuildModel>>;

/// Evaluate `gluon.rhai` from the project root and return the populated model.
pub fn evaluate_script(root: &Path) -> Result<BuildModel> {
    let model = Arc::new(Mutex::new(BuildModel::default()));
    let mut engine = Engine::new();

    // Disable features we don't need.
    engine.set_max_expr_depths(64, 64);

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
    register_helpers(&mut engine, &root_path);

    // Set up include() mechanism with circular-include detection.
    let visited_includes = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
    let script_path = root.join("gluon.rhai");
    if let Ok(canonical) = std::fs::canonicalize(&script_path) {
        visited_includes.lock().unwrap().insert(canonical);
    }
    register_include_api(&mut engine, &root_path, visited_includes);

    // Evaluate gluon.rhai with the scope containing constants.
    let ast = engine
        .compile_file(script_path.clone().into())
        .map_err(|e| anyhow::anyhow!("error compiling {}: {e}", script_path.display()))?;
    engine
        .run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| anyhow::anyhow!("error evaluating {}: {e}", script_path.display()))?;

    // Drop the engine to release all Arc references held by closures.
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
                },
            );
            ConfigGroupBuilder {
                model: m.clone(),
                group_name: name.into(),
            }
        },
    );

    // ConfigGroupBuilder methods
    engine.register_fn(
        "field",
        |builder: &mut ConfigGroupBuilder, name: &str, value: Dynamic| -> ConfigGroupBuilder {
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
            let mut model = builder.model.lock().unwrap();
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
                },
            );
            builder.clone()
        },
    );

    engine.register_fn(
        "help",
        |builder: &mut ConfigGroupBuilder, help: &str| -> ConfigGroupBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.group_name) {
                opt.help = Some(help.into());
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "menu",
        |builder: &mut ConfigGroupBuilder, menu: &str| -> ConfigGroupBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.group_name) {
                opt.menu = Some(menu.into());
            }
            // Track menu category ordering (first-appearance).
            if !model.menu_order.iter().any(|m| m == menu) {
                model.menu_order.push(menu.into());
            }
            builder.clone()
        },
    );

    // ConfigBuilder methods
    engine.register_fn(
        "depends_on",
        |builder: &mut ConfigBuilder, deps: rhai::Array| -> ConfigBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.depends_on = deps
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "selects",
        |builder: &mut ConfigBuilder, sels: rhai::Array| -> ConfigBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.selects = sels
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "range",
        |builder: &mut ConfigBuilder, min: i64, max: i64| -> ConfigBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.range = Some((min as u64, max as u64));
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "choices",
        |builder: &mut ConfigBuilder, choices: rhai::Array| -> ConfigBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.choices = Some(
                    choices
                        .into_iter()
                        .filter_map(|v| v.into_string().ok())
                        .collect(),
                );
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "help",
        |builder: &mut ConfigBuilder, help: &str| -> ConfigBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.help = Some(help.into());
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "menu",
        |builder: &mut ConfigBuilder, menu: &str| -> ConfigBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(opt) = model.config_options.get_mut(&builder.name) {
                opt.menu = Some(menu.into());
            }
            // Track menu category ordering (first-appearance).
            if !model.menu_order.iter().any(|m| m == menu) {
                model.menu_order.push(menu.into());
            }
            builder.clone()
        },
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

    engine.register_fn(
        "inherits",
        |builder: &mut ProfileBuilder, parent: &str| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.inherits = Some(parent.into());
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "target",
        |builder: &mut ProfileBuilder, target: &str| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.target = Some(target.into());
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "opt_level",
        |builder: &mut ProfileBuilder, level: i64| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.opt_level = Some(level as u32);
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "debug_info",
        |builder: &mut ProfileBuilder, enabled: bool| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.debug_info = Some(enabled);
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "lto",
        |builder: &mut ProfileBuilder, lto: &str| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.lto = Some(lto.into());
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "boot_binary",
        |builder: &mut ProfileBuilder, bin: &str| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.boot_binary = Some(bin.into());
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "config",
        |builder: &mut ProfileBuilder, overrides: Map| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                for (key, val) in overrides {
                    let config_val = dynamic_to_config_value(&val);
                    p.config.insert(key.to_string(), config_val);
                }
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "qemu_memory",
        |builder: &mut ProfileBuilder, mem: i64| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.qemu_memory = Some(mem as u32);
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "qemu_cores",
        |builder: &mut ProfileBuilder, cores: i64| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.qemu_cores = Some(cores as u32);
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "qemu_extra_args",
        |builder: &mut ProfileBuilder, args: rhai::Array| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.qemu_extra_args = Some(
                    args.into_iter()
                        .filter_map(|v| v.into_string().ok())
                        .collect(),
                );
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "test_timeout",
        |builder: &mut ProfileBuilder, timeout: i64| -> ProfileBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(p) = model.profiles.get_mut(&builder.name) {
                p.test_timeout = Some(timeout as u32);
            }
            builder.clone()
        },
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

    engine.register_fn(
        "target",
        |builder: &mut GroupBuilder, target: &str| -> GroupBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.target = target.into();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "config",
        |builder: &mut GroupBuilder, has_config: bool| -> GroupBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.config = has_config;
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "edition",
        |builder: &mut GroupBuilder, ed: &str| -> GroupBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.default_edition = ed.into();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "project",
        |builder: &mut GroupBuilder, is_proj: bool| -> GroupBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(g) = model.groups.get_mut(&builder.name) {
                g.is_project = is_proj;
            }
            builder.clone()
        },
    );

    // group.add(name, path) -> CrateBuilder
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
                },
            );
            CrateBuilder {
                model: builder.model.clone(),
                name: name.into(),
            }
        },
    );

    // CrateBuilder methods.
    engine.register_fn(
        "deps",
        |builder: &mut CrateBuilder, deps: Map| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                for (extern_name, val) in deps {
                    let dep = parse_dep_value(&extern_name, &val);
                    krate.deps.insert(extern_name.to_string(), dep);
                }
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "dev_deps",
        |builder: &mut CrateBuilder, deps: Map| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                for (extern_name, val) in deps {
                    let dep = parse_dep_value(&extern_name, &val);
                    krate.dev_deps.insert(extern_name.to_string(), dep);
                }
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "features",
        |builder: &mut CrateBuilder, feats: rhai::Array| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.features = feats
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "crate_type",
        |builder: &mut CrateBuilder, ty: i64| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.crate_type = crate_type_from_i64(ty);
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "root",
        |builder: &mut CrateBuilder, root: &str| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.root = Some(root.into());
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "edition",
        |builder: &mut CrateBuilder, ed: &str| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.edition = ed.into();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "linker_script",
        |builder: &mut CrateBuilder, script: &str| -> CrateBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(krate) = model.crates.get_mut(&builder.name) {
                krate.linker_script = Some(script.into());
            }
            builder.clone()
        },
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

    engine.register_fn(
        "inputs",
        |builder: &mut RuleBuilder, inputs: rhai::Array| -> RuleBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.inputs = inputs
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "output",
        |builder: &mut RuleBuilder, output: &str| -> RuleBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.outputs = vec![output.into()];
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "outputs",
        |builder: &mut RuleBuilder, outputs: rhai::Array| -> RuleBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.outputs = outputs
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "depends_on",
        |builder: &mut RuleBuilder, deps: rhai::Array| -> RuleBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.depends_on = deps
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect();
            }
            builder.clone()
        },
    );

    engine.register_fn(
        "handler",
        |builder: &mut RuleBuilder, handler: &str| -> RuleBuilder {
            let mut model = builder.model.lock().unwrap();
            if let Some(rule) = model.rules.get_mut(&builder.name) {
                rule.handler = RuleHandler::Builtin(handler.into());
            }
            builder.clone()
        },
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

    engine.register_fn(
        "stage",
        |builder: &mut PipelineBuilder, name: &str, groups: rhai::Array| -> PipelineBuilder {
            let mut model = builder.model.lock().unwrap();
            model.pipeline.steps.push(PipelineStep::Stage {
                name: name.into(),
                groups: groups
                    .into_iter()
                    .filter_map(|v| v.into_string().ok())
                    .collect(),
            });
            builder.clone()
        },
    );

    engine.register_fn(
        "barrier",
        |builder: &mut PipelineBuilder, name: &str| -> PipelineBuilder {
            let mut model = builder.model.lock().unwrap();
            model.pipeline.steps.push(PipelineStep::Barrier(name.into()));
            builder.clone()
        },
    );

    engine.register_fn(
        "rule",
        |builder: &mut PipelineBuilder, name: &str| -> PipelineBuilder {
            let mut model = builder.model.lock().unwrap();
            model.pipeline.steps.push(PipelineStep::Rule(name.into()));
            builder.clone()
        },
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

    engine.register_fn(
        "extra_args",
        |builder: &mut QemuBuilder, args: rhai::Array| -> QemuBuilder {
            let mut model = builder.model.lock().unwrap();
            model.qemu.extra_args = args
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
            builder.clone()
        },
    );

    engine.register_fn(
        "test_success_code",
        |builder: &mut QemuBuilder, code: i64| -> QemuBuilder {
            let mut model = builder.model.lock().unwrap();
            model.qemu.test.success_exit_code = code as u32;
            builder.clone()
        },
    );

    engine.register_fn(
        "test_timeout",
        |builder: &mut QemuBuilder, timeout: i64| -> QemuBuilder {
            let mut model = builder.model.lock().unwrap();
            model.qemu.test.timeout = timeout as u32;
            builder.clone()
        },
    );

    engine.register_fn(
        "test_extra_args",
        |builder: &mut QemuBuilder, args: rhai::Array| -> QemuBuilder {
            let mut model = builder.model.lock().unwrap();
            model.qemu.test.extra_args = args
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
            builder.clone()
        },
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

    engine.register_fn(
        "config_file",
        |builder: &mut BootloaderBuilder, file: &str| -> BootloaderBuilder {
            let mut model = builder.model.lock().unwrap();
            model.bootloader.config_file = Some(file.into());
            builder.clone()
        },
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

    engine.register_fn(
        "extra_files",
        |builder: &mut ImageBuilder, files: Map| -> ImageBuilder {
            let mut model = builder.model.lock().unwrap();
            for (key, val) in files {
                if let Ok(v) = val.into_string() {
                    model.image.extra_files.insert(key.to_string(), v);
                }
            }
            builder.clone()
        },
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

    engine.register_fn(
        "host_testable",
        |builder: &mut TestsBuilder, crates: rhai::Array| -> TestsBuilder {
            let mut model = builder.model.lock().unwrap();
            model.tests.host_testable = crates
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect();
            builder.clone()
        },
    );

    engine.register_fn(
        "kernel_tests_dir",
        |builder: &mut TestsBuilder, dir: &str| -> TestsBuilder {
            let mut model = builder.model.lock().unwrap();
            model.tests.kernel_tests_dir = Some(dir.into());
            builder.clone()
        },
    );

    engine.register_fn(
        "kernel_tests_crate",
        |builder: &mut TestsBuilder, name: &str| -> TestsBuilder {
            let mut model = builder.model.lock().unwrap();
            model.tests.kernel_tests_crate = Some(name.into());
            builder.clone()
        },
    );

    engine.register_fn(
        "kernel_tests_linker_script",
        |builder: &mut TestsBuilder, path: &str| -> TestsBuilder {
            let mut model = builder.model.lock().unwrap();
            model.tests.kernel_tests_linker_script = Some(path.into());
            builder.clone()
        },
    );
}

#[derive(Debug, Clone)]
struct TestsBuilder {
    model: SharedModel,
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
        std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                s.lines()
                    .find(|l| l.starts_with("host:"))
                    .and_then(|l| l.strip_prefix("host: "))
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "unknown".into())
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
        }
    } else {
        DepDef {
            extern_name: extern_name.into(),
            crate_name: extern_name.into(),
            features: Vec::new(),
        }
    }
}
