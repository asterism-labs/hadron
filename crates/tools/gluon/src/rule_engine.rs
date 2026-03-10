//! Rhai rule callback engine for user-defined build rules.
//!
//! Provides [`RuleContext`] as a Rhai custom type and a set of `gluon::*`
//! helper functions that wrap existing artifact generation routines. This
//! enables rules defined in `gluon.rhai` to perform file operations, invoke
//! artifact generators, and trigger re-linking — all from Rhai script code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::Result;
use rhai::{Dynamic, Engine, Module};

use crate::compile::ArtifactMap;
use crate::config::ResolvedConfig;
use crate::model::{BuildModel, CrateType, RuleDef};

/// Context passed to Rhai rule callback scripts as the `ctx` variable.
#[derive(Debug, Clone)]
pub struct RuleContext {
    /// Name of the rule being executed.
    pub rule_name: String,
    /// Whether any of the rule's input crates were rebuilt in this session.
    pub inputs_rebuilt: bool,
    /// Whether `--force` was passed.
    pub force: bool,
    /// Whether debug info is enabled for this profile.
    pub debug_info: bool,
    /// Absolute path to the build directory (e.g. `<root>/build`).
    pub build_dir: String,
    /// Absolute path to the target output directory (e.g. `<root>/target`).
    pub target_dir: String,
    /// Absolute path to the project root.
    pub root_dir: String,
    /// Name of the active profile.
    pub profile_name: String,
    /// Name of the boot binary crate.
    pub boot_binary: String,
    /// Name of the active target.
    pub target_name: String,
    /// Input crate name → artifact path.
    inputs: BTreeMap<String, String>,
    /// All artifacts snapshot (crate name → artifact path).
    artifacts: BTreeMap<String, String>,
    /// Set by `gluon::set_kernel_binary` to override the kernel binary path.
    pub kernel_binary_override: Option<String>,
}

impl RuleContext {
    /// Create a new rule context from pipeline state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rule_name: &str,
        rule: &RuleDef,
        config: &ResolvedConfig,
        root: &Path,
        kernel_binary: &Option<PathBuf>,
        kernel_binary_rebuilt: bool,
        force: bool,
        shared_artifacts: &RwLock<ArtifactMap>,
    ) -> Self {
        let arts = shared_artifacts.read().unwrap();
        let mut inputs = BTreeMap::new();

        for input in &rule.inputs {
            if let Some(path) = arts.get(input) {
                inputs.insert(input.clone(), path.display().to_string());
            }
        }

        let mut all_artifacts = BTreeMap::new();

        // Add kernel binary if available.
        if let Some(kb) = kernel_binary {
            all_artifacts.insert(config.profile.boot_binary.clone(), kb.display().to_string());
        }

        // Merge inputs into all_artifacts.
        for (k, v) in &inputs {
            all_artifacts.insert(k.clone(), v.clone());
        }

        drop(arts);

        Self {
            rule_name: rule_name.into(),
            inputs_rebuilt: kernel_binary_rebuilt,
            force,
            debug_info: config.profile.debug_info,
            build_dir: root.join("build").display().to_string(),
            target_dir: root.join("target").display().to_string(),
            root_dir: root.display().to_string(),
            profile_name: config.profile.name.clone(),
            boot_binary: config.profile.boot_binary.clone(),
            target_name: config.profile.target.clone(),
            inputs,
            artifacts: all_artifacts,
            kernel_binary_override: None,
        }
    }
}

/// Register `RuleContext` type and its getters/methods on the Rhai engine.
fn register_rule_context(engine: &mut Engine) {
    engine
        .register_type_with_name::<RuleContext>("RuleContext")
        .register_get("rule_name", |ctx: &mut RuleContext| ctx.rule_name.clone())
        .register_get("inputs_rebuilt", |ctx: &mut RuleContext| ctx.inputs_rebuilt)
        .register_get("force", |ctx: &mut RuleContext| ctx.force)
        .register_get("debug_info", |ctx: &mut RuleContext| ctx.debug_info)
        .register_get("build_dir", |ctx: &mut RuleContext| ctx.build_dir.clone())
        .register_get("target_dir", |ctx: &mut RuleContext| ctx.target_dir.clone())
        .register_get("root_dir", |ctx: &mut RuleContext| ctx.root_dir.clone())
        .register_get("profile_name", |ctx: &mut RuleContext| {
            ctx.profile_name.clone()
        })
        .register_get("boot_binary", |ctx: &mut RuleContext| {
            ctx.boot_binary.clone()
        })
        .register_get("target_name", |ctx: &mut RuleContext| {
            ctx.target_name.clone()
        });

    // ctx.input(name) -> path string
    engine.register_fn("input", |ctx: &mut RuleContext, name: &str| -> String {
        ctx.inputs.get(name).cloned().unwrap_or_default()
    });

    // ctx.artifact(name) -> path string
    engine.register_fn("artifact", |ctx: &mut RuleContext, name: &str| -> String {
        ctx.artifacts.get(name).cloned().unwrap_or_default()
    });

    // ctx.bin_artifacts() -> array of [name, path] pairs
    engine.register_fn("bin_artifacts", |ctx: &mut RuleContext| -> rhai::Array {
        ctx.inputs
            .iter()
            .map(|(name, path)| {
                Dynamic::from(vec![
                    Dynamic::from(name.clone()),
                    Dynamic::from(path.clone()),
                ])
            })
            .collect()
    });
}

/// Create a `gluon::*` Rhai module with helper functions.
fn create_gluon_module(config: &ResolvedConfig) -> Module {
    let mut module = Module::new();

    // gluon::generate_hbtf(kernel, output, debug_info)
    module.set_native_fn(
        "generate_hbtf",
        |kernel: &str,
         output: &str,
         debug_info: bool|
         -> std::result::Result<(), Box<rhai::EvalAltResult>> {
            crate::artifact::hbtf::generate_hbtf(Path::new(kernel), Path::new(output), debug_info)
                .map_err(|e| format!("generate_hbtf failed: {e}").into())
        },
    );

    // gluon::generate_hkif(kernel, output, debug_info)
    module.set_native_fn(
        "generate_hkif",
        |kernel: &str,
         output: &str,
         debug_info: bool|
         -> std::result::Result<(), Box<rhai::EvalAltResult>> {
            crate::artifact::hkif::generate_hkif(Path::new(kernel), Path::new(output), debug_info)
                .map_err(|e| format!("generate_hkif failed: {e}").into())
        },
    );

    // gluon::generate_hkif_asm(hkif_bin, output)
    module.set_native_fn(
        "generate_hkif_asm",
        |hkif_bin: &str, output: &str| -> std::result::Result<(), Box<rhai::EvalAltResult>> {
            crate::artifact::hkif::generate_hkif_asm(Path::new(hkif_bin), Path::new(output))
                .map_err(|e| format!("generate_hkif_asm failed: {e}").into())
        },
    );

    // gluon::assemble_hkif(asm, obj)
    module.set_native_fn(
        "assemble_hkif",
        |asm: &str, obj: &str| -> std::result::Result<(), Box<rhai::EvalAltResult>> {
            crate::artifact::hkif::assemble_hkif(Path::new(asm), Path::new(obj))
                .map_err(|e| format!("assemble_hkif failed: {e}").into())
        },
    );

    // gluon::hkif_object_path(root)
    module.set_native_fn(
        "hkif_object_path",
        |root: &str| -> std::result::Result<String, Box<rhai::EvalAltResult>> {
            Ok(crate::artifact::hkif::hkif_object_path(Path::new(root))
                .display()
                .to_string())
        },
    );

    // gluon::copy(src, dst)
    module.set_native_fn(
        "copy",
        |src: &str, dst: &str| -> std::result::Result<(), Box<rhai::EvalAltResult>> {
            let dst_path = Path::new(dst);
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| -> Box<rhai::EvalAltResult> {
                    format!("mkdir for copy failed: {e}").into()
                })?;
            }
            std::fs::copy(src, dst)
                .map_err(|e| -> Box<rhai::EvalAltResult> { format!("copy failed: {e}").into() })?;
            Ok(())
        },
    );

    // gluon::mkdir(path)
    module.set_native_fn(
        "mkdir",
        |path: &str| -> std::result::Result<(), Box<rhai::EvalAltResult>> {
            std::fs::create_dir_all(path).map_err(|e| format!("mkdir failed: {e}").into())
        },
    );

    // gluon::println(msg)
    module.set_native_fn(
        "println",
        |msg: &str| -> std::result::Result<(), Box<rhai::EvalAltResult>> {
            println!("{msg}");
            Ok(())
        },
    );

    // gluon::build_cpio(bin_artifacts_array) - uses captured config root
    {
        let config_clone = config.clone();
        module.set_native_fn("build_cpio", move |bin_artifacts: rhai::Array| -> std::result::Result<String, Box<rhai::EvalAltResult>> {
            let mut artifacts = Vec::new();
            for item in bin_artifacts {
                let pair = item.into_typed_array::<Dynamic>()
                    .map_err(|e| -> Box<rhai::EvalAltResult> { format!("build_cpio: invalid artifact pair: {e}").into() })?;
                if pair.len() >= 2 {
                    let name = pair[0].clone().into_string()
                        .map_err(|e| -> Box<rhai::EvalAltResult> { format!("build_cpio: invalid name: {e}").into() })?;
                    let path = pair[1].clone().into_string()
                        .map_err(|e| -> Box<rhai::EvalAltResult> { format!("build_cpio: invalid path: {e}").into() })?;
                    artifacts.push((name, PathBuf::from(path)));
                }
            }
            let result = crate::artifact::initrd::build_initrd(&config_clone, &artifacts)
                .map_err(|e| -> Box<rhai::EvalAltResult> { format!("build_cpio failed: {e}").into() })?;
            Ok(result.display().to_string())
        });
    }

    module
}

/// Create a Rhai engine configured for rule script execution.
///
/// Registers the `RuleContext` type, `gluon::*` helper module, and standard
/// Rhai packages.
pub fn create_rule_engine(
    _model: &BuildModel,
    config: &ResolvedConfig,
    _shared_artifacts: &RwLock<ArtifactMap>,
    _shared_target_specs: &RwLock<std::collections::HashMap<String, String>>,
    _shared_sysroots: &RwLock<std::collections::HashMap<String, PathBuf>>,
    _shared_config_rlibs: &RwLock<std::collections::HashMap<String, PathBuf>>,
) -> Engine {
    let mut engine = Engine::new();
    register_rule_context(&mut engine);

    // Register gluon::set_kernel_binary as a direct engine function
    // (not on the module, since it takes &mut RuleContext).
    engine.register_fn("set_kernel_binary", |ctx: &mut RuleContext, path: &str| {
        ctx.kernel_binary_override = Some(path.into());
    });

    // Register the gluon:: module functions on a static module.
    let gluon_module = create_gluon_module(config);
    engine.register_static_module("gluon", gluon_module.into());

    engine
}

/// Execute a Rhai rule script and return the (potentially mutated) context.
pub fn execute_script(
    engine: &Engine,
    source: &str,
    rule_name: &str,
    ctx: RuleContext,
) -> Result<RuleContext> {
    let ast = engine
        .compile(source)
        .map_err(|e| anyhow::anyhow!("rule '{rule_name}' compile error: {e}"))?;

    let mut scope = rhai::Scope::new();
    scope.push("ctx", ctx);

    engine
        .run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| anyhow::anyhow!("rule '{rule_name}' execution error: {e}"))?;

    let ctx: RuleContext = scope.get_value("ctx").ok_or_else(|| {
        anyhow::anyhow!("rule '{rule_name}': ctx variable missing after execution")
    })?;

    Ok(ctx)
}

/// Filter bin artifacts from a model's crate definitions and artifact map.
#[allow(dead_code)] // used when migrating initrd rule to Rhai
pub fn collect_bin_artifacts(
    model: &BuildModel,
    rule: &RuleDef,
    shared_artifacts: &RwLock<ArtifactMap>,
) -> Vec<(String, PathBuf)> {
    let arts = shared_artifacts.read().unwrap();
    let mut bin_artifacts = Vec::new();
    for input in &rule.inputs {
        if let (Some(path), Some(def)) = (arts.get(input), model.crates.get(input)) {
            if def.crate_type == CrateType::Bin {
                bin_artifacts.push((input.clone(), path.to_path_buf()));
            }
        }
    }
    bin_artifacts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RuleHandler;

    fn make_test_config(root: &Path) -> ResolvedConfig {
        use crate::config::*;
        ResolvedConfig {
            project: ProjectMeta {
                name: "test".into(),
                version: "0.1.0".into(),
            },
            root: root.to_path_buf(),
            target_name: "host".into(),
            target: TargetConfig {
                spec: "x86_64-unknown-linux-gnu".into(),
            },
            options: std::collections::BTreeMap::new(),
            bindings: std::collections::BTreeMap::new(),
            choices: std::collections::BTreeMap::new(),
            profile: ResolvedProfile {
                name: "test".into(),
                target: "host".into(),
                opt_level: 0,
                debug_info: false,
                lto: None,
                boot_binary: "test-kernel".into(),
                qemu_memory: None,
                qemu_cores: None,
                qemu_extra_args: None,
                test_timeout: None,
            },
            qemu: QemuConfig {
                machine: "q35".into(),
                memory: 256,
                extra_args: vec![],
                test: QemuTestConfig {
                    success_exit_code: 33,
                    timeout: 30,
                    extra_args: vec![],
                },
            },
            bootloader: BootloaderConfig {
                kind: "limine".into(),
                config_file: None,
            },
            image: ImageConfig::default(),
            tests: TestsConfig::default(),
            benchmarks: BenchmarksConfig::default(),
        }
    }

    fn make_test_rule(inputs: &[&str]) -> RuleDef {
        RuleDef {
            name: "test_rule".into(),
            inputs: inputs.iter().map(|s| s.to_string()).collect(),
            outputs: vec![],
            depends_on: vec![],
            handler: RuleHandler::Script(String::new()),
        }
    }

    fn make_test_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gluon_rule_test_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    #[test]
    fn rule_context_creation() {
        let dir = make_test_dir("ctx");
        let config = make_test_config(&dir);
        let rule = make_test_rule(&["input_a"]);
        let artifacts = RwLock::new(ArtifactMap::default());
        artifacts
            .write()
            .unwrap()
            .insert("input_a", dir.join("a.bin"));

        let ctx = RuleContext::new(
            "test_rule",
            &rule,
            &config,
            &dir,
            &None,
            false,
            false,
            &artifacts,
        );

        assert_eq!(ctx.rule_name, "test_rule");
        assert!(!ctx.inputs_rebuilt);
        assert!(!ctx.force);
        assert!(ctx.inputs.get("input_a").unwrap().contains("a.bin"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rule_context_input_method() {
        let dir = make_test_dir("input");
        let config = make_test_config(&dir);
        let rule = make_test_rule(&["my_input"]);
        let artifacts = RwLock::new(ArtifactMap::default());
        artifacts
            .write()
            .unwrap()
            .insert("my_input", dir.join("my.bin"));

        let mut ctx = RuleContext::new(
            "test", &rule, &config, &dir, &None, false, false, &artifacts,
        );

        assert!(ctx.inputs.get("my_input").unwrap().contains("my.bin"));
        assert!(ctx.inputs.get("nonexistent").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_execution_basic() {
        let dir = make_test_dir("exec");
        let config = make_test_config(&dir);
        let model = BuildModel::default();
        let artifacts = RwLock::new(ArtifactMap::default());
        let target_specs = RwLock::new(std::collections::HashMap::new());
        let sysroots = RwLock::new(std::collections::HashMap::new());
        let config_rlibs = RwLock::new(std::collections::HashMap::new());

        let engine = create_rule_engine(
            &model,
            &config,
            &artifacts,
            &target_specs,
            &sysroots,
            &config_rlibs,
        );

        let rule = make_test_rule(&[]);
        let ctx = RuleContext::new(
            "test", &rule, &config, &dir, &None, false, false, &artifacts,
        );

        let result = execute_script(&engine, r#"let bd = ctx.build_dir;"#, "test", ctx);
        assert!(
            result.is_ok(),
            "basic script should execute: {}",
            result.unwrap_err()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_execution_with_copy() {
        let dir = make_test_dir("copy");
        let src_file = dir.join("source.txt");
        std::fs::write(&src_file, "hello").unwrap();
        let dst_file = dir.join("subdir/dest.txt");

        let config = make_test_config(&dir);
        let model = BuildModel::default();
        let artifacts = RwLock::new(ArtifactMap::default());
        let target_specs = RwLock::new(std::collections::HashMap::new());
        let sysroots = RwLock::new(std::collections::HashMap::new());
        let config_rlibs = RwLock::new(std::collections::HashMap::new());

        let engine = create_rule_engine(
            &model,
            &config,
            &artifacts,
            &target_specs,
            &sysroots,
            &config_rlibs,
        );

        let rule = make_test_rule(&[]);
        let ctx = RuleContext::new(
            "test", &rule, &config, &dir, &None, false, false, &artifacts,
        );

        let script = format!(
            r#"gluon::copy("{}", "{}");"#,
            src_file.display(),
            dst_file.display(),
        );
        let result = execute_script(&engine, &script, "test", ctx);
        assert!(
            result.is_ok(),
            "copy should succeed: {}",
            result.unwrap_err()
        );
        assert!(
            dst_file.exists(),
            "destination file should exist after copy"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_compile_error_reporting() {
        let dir = make_test_dir("compile_err");
        let config = make_test_config(&dir);
        let model = BuildModel::default();
        let artifacts = RwLock::new(ArtifactMap::default());
        let target_specs = RwLock::new(std::collections::HashMap::new());
        let sysroots = RwLock::new(std::collections::HashMap::new());
        let config_rlibs = RwLock::new(std::collections::HashMap::new());

        let engine = create_rule_engine(
            &model,
            &config,
            &artifacts,
            &target_specs,
            &sysroots,
            &config_rlibs,
        );

        let rule = make_test_rule(&[]);
        let ctx = RuleContext::new(
            "test", &rule, &config, &dir, &None, false, false, &artifacts,
        );

        let result = execute_script(&engine, "this is not valid {{ rhai", "my_rule", ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("my_rule"),
            "error should include rule name, got: {err_msg}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn script_runtime_error_reporting() {
        let dir = make_test_dir("runtime_err");
        let config = make_test_config(&dir);
        let model = BuildModel::default();
        let artifacts = RwLock::new(ArtifactMap::default());
        let target_specs = RwLock::new(std::collections::HashMap::new());
        let sysroots = RwLock::new(std::collections::HashMap::new());
        let config_rlibs = RwLock::new(std::collections::HashMap::new());

        let engine = create_rule_engine(
            &model,
            &config,
            &artifacts,
            &target_specs,
            &sysroots,
            &config_rlibs,
        );

        let rule = make_test_rule(&[]);
        let ctx = RuleContext::new(
            "test", &rule, &config, &dir, &None, false, false, &artifacts,
        );

        let result = execute_script(&engine, "nonexistent_function();", "my_rule", ctx);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("my_rule"),
            "error should include rule name, got: {err_msg}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kernel_binary_override() {
        let dir = make_test_dir("kb_override");
        let config = make_test_config(&dir);
        let model = BuildModel::default();
        let artifacts = RwLock::new(ArtifactMap::default());
        let target_specs = RwLock::new(std::collections::HashMap::new());
        let sysroots = RwLock::new(std::collections::HashMap::new());
        let config_rlibs = RwLock::new(std::collections::HashMap::new());

        let engine = create_rule_engine(
            &model,
            &config,
            &artifacts,
            &target_specs,
            &sysroots,
            &config_rlibs,
        );

        let rule = make_test_rule(&[]);
        let ctx = RuleContext::new(
            "test", &rule, &config, &dir, &None, false, false, &artifacts,
        );

        let script = r#"set_kernel_binary(ctx, "/new/kernel/path");"#;
        let result = execute_script(&engine, script, "test", ctx).unwrap();
        assert_eq!(
            result.kernel_binary_override,
            Some("/new/kernel/path".into())
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
