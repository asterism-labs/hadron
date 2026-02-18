//! Configuration resolution for hadron-build.
//!
//! Resolves a [`BuildModel`] (produced by Rhai script evaluation) into a
//! [`ResolvedConfig`] by applying profile inheritance, config option
//! dependencies (`select`, `depends-on`, `range`, `choices`), and target
//! resolution.

use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::model::{BuildModel, ConfigType, ConfigValue};

// ===========================================================================
// Shared types (consumed by compile, run, test, analyzer, scheduler, etc.)
// ===========================================================================

/// Project metadata.
#[derive(Debug)]
pub struct ProjectMeta {
    pub name: String,
    pub version: String,
}

/// A compilation target definition.
#[derive(Debug)]
pub struct TargetConfig {
    pub spec: String,
}

/// QEMU configuration.
#[derive(Debug)]
pub struct QemuConfig {
    pub machine: String,
    pub memory: u32,
    pub extra_args: Vec<String>,
    pub test: QemuTestConfig,
}

/// QEMU test configuration.
#[derive(Debug)]
pub struct QemuTestConfig {
    pub success_exit_code: u32,
    pub timeout: u32,
    pub extra_args: Vec<String>,
}

/// Bootloader configuration.
#[derive(Debug)]
pub struct BootloaderConfig {
    pub kind: String,
    pub config_file: Option<String>,
}

/// Image configuration.
#[derive(Debug, Default)]
pub struct ImageConfig {
    pub extra_files: BTreeMap<String, String>,
}

/// Test configuration.
#[derive(Debug, Default)]
pub struct TestsConfig {
    pub host_testable: Vec<String>,
    pub kernel_tests_dir: Option<String>,
    pub crash: Vec<CrashTest>,
}

/// A crash test definition.
#[derive(Debug)]
pub struct CrashTest {
    pub name: String,
    pub source: String,
    pub expected_exit: u32,
    pub expect_output: Option<String>,
}

/// Resolved configuration after validation and dependency resolution.
#[derive(Debug)]
pub struct ResolvedConfig {
    pub project: ProjectMeta,
    pub root: PathBuf,
    pub target_name: String,
    pub target: TargetConfig,
    pub options: BTreeMap<String, ResolvedValue>,
    pub profile: ResolvedProfile,
    pub qemu: QemuConfig,
    #[allow(dead_code)] // used by image generation
    pub bootloader: BootloaderConfig,
    #[allow(dead_code)] // used by image generation
    pub image: ImageConfig,
    pub tests: TestsConfig,
}

/// A fully resolved build profile.
#[derive(Debug)]
pub struct ResolvedProfile {
    pub name: String,
    pub target: String,
    pub opt_level: u32,
    pub debug_info: bool,
    pub lto: Option<String>,
    pub boot_binary: String,
    pub qemu_memory: Option<u32>,
    pub qemu_cores: Option<u32>,
    pub qemu_extra_args: Option<Vec<String>>,
    pub test_timeout: Option<u32>,
}

/// A resolved config value.
#[derive(Debug, Clone)]
pub enum ResolvedValue {
    Bool(bool),
    U32(u32),
    U64(u64),
    Str(String),
}

impl std::fmt::Display for ResolvedValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolvedValue::Bool(v) => write!(f, "{v}"),
            ResolvedValue::U32(v) => write!(f, "{v}"),
            ResolvedValue::U64(v) => write!(f, "{v:#x}"),
            ResolvedValue::Str(v) => write!(f, "{v}"),
        }
    }
}

// ===========================================================================
// Project root discovery
// ===========================================================================

/// Find the project root by looking for `build.rhai`.
pub fn find_project_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("build.rhai").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("could not find build.rhai in any parent directory");
        }
    }
}

// ===========================================================================
// BuildModel → ResolvedConfig resolution
// ===========================================================================

/// Resolve configuration from a [`BuildModel`] (produced by Rhai evaluation).
///
/// Applies profile inheritance, resolves config options (select/depends-on),
/// and produces a [`ResolvedConfig`] consumed by the rest of the codebase.
pub fn resolve_from_model(
    model: &BuildModel,
    profile_name: &str,
    target_override: Option<&str>,
    root: &Path,
) -> Result<ResolvedConfig> {
    // Resolve profile with inheritance.
    let profile = resolve_profile(model, profile_name)?;

    // Target from override > profile > first in targets map.
    let target_name = target_override
        .map(String::from)
        .or_else(|| Some(profile.target.clone()))
        .unwrap();

    let target_def = model
        .targets
        .get(&target_name)
        .with_context(|| format!("target '{target_name}' not found in build model"))?;

    // Collect merged config overrides from the profile inheritance chain.
    let profile_overrides = collect_profile_config(model, profile_name)?;

    // Load .hadron-config overrides (user-level, from menuconfig).
    let file_overrides = load_config_overrides(root)?;

    // Merge: profile overrides first, then .hadron-config on top.
    let mut merged_overrides = profile_overrides;
    merged_overrides.extend(file_overrides);

    // Resolve config options with merged overrides.
    let options = resolve_options(
        &model.config_options,
        &merged_overrides,
    )?;

    // Apply select/depends-on validation.
    let options = apply_selects_and_validate(
        options,
        &model.config_options,
    )?;

    let resolved_target = TargetConfig {
        spec: target_def.spec.clone(),
    };

    let qemu = QemuConfig {
        machine: model.qemu.machine.clone(),
        memory: model.qemu.memory,
        extra_args: model.qemu.extra_args.clone(),
        test: QemuTestConfig {
            success_exit_code: model.qemu.test.success_exit_code,
            timeout: model.qemu.test.timeout,
            extra_args: model.qemu.test.extra_args.clone(),
        },
    };

    let bootloader = BootloaderConfig {
        kind: model.bootloader.kind.clone(),
        config_file: model.bootloader.config_file.clone(),
    };

    let image = ImageConfig {
        extra_files: model.image.extra_files.clone(),
    };

    let tests = TestsConfig {
        host_testable: model.tests.host_testable.clone(),
        kernel_tests_dir: model.tests.kernel_tests_dir.clone(),
        crash: model.tests.crash_tests.iter().map(|ct| CrashTest {
            name: ct.name.clone(),
            source: ct.source.clone(),
            expected_exit: ct.expected_exit,
            expect_output: ct.expect_output.clone(),
        }).collect(),
    };

    Ok(ResolvedConfig {
        project: ProjectMeta {
            name: model.project.name.clone(),
            version: model.project.version.clone(),
        },
        root: root.to_path_buf(),
        target_name,
        target: resolved_target,
        options,
        profile,
        qemu,
        bootloader,
        image,
        tests,
    })
}

/// Collect merged config overrides from a profile's inheritance chain.
/// Parent config is applied first, then child overrides on top.
fn collect_profile_config(
    model: &BuildModel,
    name: &str,
) -> Result<BTreeMap<String, ConfigValue>> {
    let profile = model.profiles.get(name)
        .with_context(|| format!("profile '{name}' not found in build model"))?;

    let mut merged = if let Some(ref parent_name) = profile.inherits {
        collect_profile_config(model, parent_name)?
    } else {
        BTreeMap::new()
    };

    merged.extend(profile.config.clone());
    Ok(merged)
}

/// Resolve a profile by applying its inheritance chain.
fn resolve_profile(
    model: &BuildModel,
    name: &str,
) -> Result<ResolvedProfile> {
    let profile = model.profiles.get(name)
        .with_context(|| format!("profile '{name}' not found in build model"))?;

    let parent = if let Some(ref parent_name) = profile.inherits {
        Some(resolve_profile(model, parent_name)?)
    } else {
        None
    };

    let target = profile.target.clone()
        .or_else(|| parent.as_ref().map(|p| p.target.clone()))
        .or_else(|| model.targets.keys().next().cloned())
        .context("profile has no target and no targets defined")?;

    let opt_level = profile.opt_level
        .or(parent.as_ref().map(|p| p.opt_level))
        .unwrap_or(0);

    let debug_info = profile.debug_info
        .or(parent.as_ref().map(|p| p.debug_info))
        .unwrap_or(true);

    let lto = profile.lto.clone()
        .or_else(|| parent.as_ref().and_then(|p| p.lto.clone()));

    let boot_binary = profile.boot_binary.clone()
        .or_else(|| parent.as_ref().map(|p| p.boot_binary.clone()))
        .unwrap_or_else(|| "hadron-boot-limine".into());

    let qemu_memory = profile.qemu_memory
        .or(parent.as_ref().and_then(|p| p.qemu_memory));

    let qemu_cores = profile.qemu_cores
        .or(parent.as_ref().and_then(|p| p.qemu_cores));

    let qemu_extra_args = profile.qemu_extra_args.clone()
        .or_else(|| parent.as_ref().and_then(|p| p.qemu_extra_args.clone()));

    let test_timeout = profile.test_timeout
        .or(parent.as_ref().and_then(|p| p.test_timeout));

    Ok(ResolvedProfile {
        name: name.into(),
        target,
        opt_level,
        debug_info,
        lto,
        boot_binary,
        qemu_memory,
        qemu_cores,
        qemu_extra_args,
        test_timeout,
    })
}

/// Resolve config option values, applying profile overrides.
fn resolve_options(
    options: &BTreeMap<String, crate::model::ConfigOptionDef>,
    profile_overrides: &BTreeMap<String, ConfigValue>,
) -> Result<BTreeMap<String, ResolvedValue>> {
    let mut resolved = BTreeMap::new();

    for (name, opt) in options {
        let value = profile_overrides
            .get(name)
            .unwrap_or(&opt.default);

        let resolved_value = match (&opt.ty, value) {
            (ConfigType::Bool, ConfigValue::Bool(v)) => ResolvedValue::Bool(*v),
            (ConfigType::U32, ConfigValue::U32(v)) => ResolvedValue::U32(*v),
            (ConfigType::U64, ConfigValue::U64(v)) => ResolvedValue::U64(*v),
            (ConfigType::Str, ConfigValue::Str(v)) => ResolvedValue::Str(v.clone()),
            _ => bail!(
                "config option '{name}' value type does not match declared type {:?}",
                opt.ty
            ),
        };

        // Validate range.
        if let Some((min, max)) = opt.range {
            match &resolved_value {
                ResolvedValue::U32(v) => ensure!(
                    u64::from(*v) >= min && u64::from(*v) <= max,
                    "option '{name}' value {v} outside range [{min}, {max}]"
                ),
                ResolvedValue::U64(v) => ensure!(
                    *v >= min && *v <= max,
                    "option '{name}' value {v:#x} outside range [{min:#x}, {max:#x}]"
                ),
                _ => bail!("range constraint on non-numeric option '{name}'"),
            }
        }

        // Validate choices.
        if let Some(ref choices) = opt.choices {
            if let ResolvedValue::Str(ref v) = resolved_value {
                ensure!(
                    choices.contains(v),
                    "option '{name}' value '{v}' not in choices: {choices:?}"
                );
            }
        }

        resolved.insert(name.clone(), resolved_value);
    }

    Ok(resolved)
}

/// Apply `select` and validate `depends-on` constraints.
fn apply_selects_and_validate(
    mut options: BTreeMap<String, ResolvedValue>,
    defs: &BTreeMap<String, crate::model::ConfigOptionDef>,
) -> Result<BTreeMap<String, ResolvedValue>> {
    // Apply selects transitively.
    loop {
        let mut changed = false;
        for (name, def) in defs {
            let is_enabled = matches!(options.get(name), Some(ResolvedValue::Bool(true)));
            if is_enabled {
                for selected in &def.selects {
                    if let Some(ResolvedValue::Bool(false)) = options.get(selected) {
                        options.insert(selected.clone(), ResolvedValue::Bool(true));
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Validate depends-on.
    for (name, def) in defs {
        let is_enabled = matches!(options.get(name), Some(ResolvedValue::Bool(true)));
        if is_enabled {
            for dep in &def.depends_on {
                match options.get(dep) {
                    Some(ResolvedValue::Bool(true)) => {}
                    Some(ResolvedValue::Bool(false)) => {
                        bail!(
                            "option '{name}' depends on '{dep}', but '{dep}' is disabled"
                        );
                    }
                    _ => {
                        ensure!(
                            options.contains_key(dep),
                            "option '{name}' depends on '{dep}', which is not defined"
                        );
                    }
                }
            }
        }
    }

    Ok(options)
}

// ===========================================================================
// Utilities
// ===========================================================================

/// Load config overrides from `.hadron-config` (key = value lines, `#` comments).
///
/// Returns an empty map if the file does not exist.
pub fn load_config_overrides(root: &Path) -> Result<BTreeMap<String, ConfigValue>> {
    let path = root.join(".hadron-config");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;

    let mut overrides = BTreeMap::new();
    for (lineno, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            bail!(".hadron-config:{}: invalid line (expected KEY=VALUE): {line}", lineno + 1);
        };
        let key = key.trim().to_string();
        let val = val.trim();

        let config_val = if val == "true" {
            ConfigValue::Bool(true)
        } else if val == "false" {
            ConfigValue::Bool(false)
        } else if let Some(hex) = val.strip_prefix("0x") {
            let parsed = u64::from_str_radix(&hex.replace('_', ""), 16)
                .with_context(|| format!(".hadron-config:{}: invalid hex value: {val}", lineno + 1))?;
            ConfigValue::U64(parsed)
        } else if let Ok(v) = val.parse::<u32>() {
            ConfigValue::U32(v)
        } else {
            ConfigValue::Str(val.to_string())
        };

        overrides.insert(key, config_val);
    }

    Ok(overrides)
}

/// Save config values to `.hadron-config`.
pub fn save_config_overrides(
    root: &Path,
    values: &BTreeMap<String, ConfigValue>,
) -> Result<()> {
    let path = root.join(".hadron-config");
    let mut content = String::new();
    content.push_str("# Generated by hadron-build menuconfig\n");
    for (key, val) in values {
        let val_str = match val {
            ConfigValue::Bool(v) => format!("{v}"),
            ConfigValue::U32(v) => format!("{v}"),
            ConfigValue::U64(v) => format!("{v:#x}"),
            ConfigValue::Str(v) => v.clone(),
        };
        content.push_str(&format!("{key} = {val_str}\n"));
    }
    std::fs::write(&path, content)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Print resolved config to stdout for debugging.
pub fn print_resolved(config: &ResolvedConfig) {
    println!("Project: {} v{}", config.project.name, config.project.version);
    println!("Target: {}", config.target_name);
    println!("  spec: {}", config.target.spec);
    println!("Profile: {}", config.profile.name);
    println!("  opt-level: {}", config.profile.opt_level);
    println!("  debug-info: {}", config.profile.debug_info);
    if let Some(ref lto) = config.profile.lto {
        println!("  lto: {lto}");
    }
    println!("  boot-binary: {}", config.profile.boot_binary);
    println!("\nResolved config options:");
    for (name, value) in &config.options {
        println!("  {name} = {value}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ConfigOptionDef;

    #[test]
    fn select_enables_dependency() {
        let mut options = BTreeMap::new();
        options.insert("smp".into(), ResolvedValue::Bool(true));
        options.insert("apic".into(), ResolvedValue::Bool(false));
        options.insert("acpi".into(), ResolvedValue::Bool(true));

        let mut defs = BTreeMap::new();
        defs.insert("smp".into(), ConfigOptionDef {
            name: "smp".into(),
            ty: ConfigType::Bool,
            default: ConfigValue::Bool(false),
            help: None,
            depends_on: vec!["acpi".into()],
            selects: vec!["apic".into()],
            range: None,
            choices: None,
            menu: None,
        });
        defs.insert("apic".into(), ConfigOptionDef {
            name: "apic".into(),
            ty: ConfigType::Bool,
            default: ConfigValue::Bool(false),
            help: None,
            depends_on: vec![],
            selects: vec![],
            range: None,
            choices: None,
            menu: None,
        });
        defs.insert("acpi".into(), ConfigOptionDef {
            name: "acpi".into(),
            ty: ConfigType::Bool,
            default: ConfigValue::Bool(true),
            help: None,
            depends_on: vec![],
            selects: vec![],
            range: None,
            choices: None,
            menu: None,
        });

        let result = apply_selects_and_validate(options, &defs).unwrap();
        assert!(matches!(result.get("apic"), Some(ResolvedValue::Bool(true))));
    }

    #[test]
    fn depends_on_fails_when_dep_disabled() {
        let mut options = BTreeMap::new();
        options.insert("smp".into(), ResolvedValue::Bool(true));
        options.insert("acpi".into(), ResolvedValue::Bool(false));

        let mut defs = BTreeMap::new();
        defs.insert("smp".into(), ConfigOptionDef {
            name: "smp".into(),
            ty: ConfigType::Bool,
            default: ConfigValue::Bool(false),
            help: None,
            depends_on: vec!["acpi".into()],
            selects: vec![],
            range: None,
            choices: None,
            menu: None,
        });
        defs.insert("acpi".into(), ConfigOptionDef {
            name: "acpi".into(),
            ty: ConfigType::Bool,
            default: ConfigValue::Bool(true),
            help: None,
            depends_on: vec![],
            selects: vec![],
            range: None,
            choices: None,
            menu: None,
        });

        let result = apply_selects_and_validate(options, &defs);
        assert!(result.is_err());
    }
}
