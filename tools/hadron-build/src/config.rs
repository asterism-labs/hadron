//! Configuration parsing and validation for hadron-build.
//!
//! Parses `hadron.toml` from the project root, resolves config option
//! dependencies (`select`, `depends-on`, `range`, `choices`), and handles
//! profile inheritance.

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Top-level configuration loaded from `hadron.toml`.
#[derive(Debug, Deserialize)]
pub struct ProjectConfig {
    pub project: ProjectMeta,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
    #[serde(default)]
    pub config: ConfigSection,
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileConfig>,
    #[serde(default)]
    pub qemu: QemuConfig,
    #[serde(default)]
    pub bootloader: BootloaderConfig,
    #[serde(default)]
    pub image: ImageConfig,
    #[serde(default)]
    pub tests: TestsConfig,
}

/// `[project]` section.
#[derive(Debug, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub version: String,
}

/// `[targets.<name>]` entry.
#[derive(Debug, Deserialize)]
pub struct TargetConfig {
    pub spec: String,
    #[serde(rename = "linker-script")]
    pub linker_script: Option<String>,
}

/// `[config]` section containing options.
#[derive(Debug, Default, Deserialize)]
pub struct ConfigSection {
    #[serde(default)]
    pub options: BTreeMap<String, ConfigOption>,
}

/// A single configuration option.
#[derive(Debug, Deserialize)]
pub struct ConfigOption {
    #[serde(rename = "type")]
    pub ty: String,
    pub default: toml::Value,
    #[serde(default)]
    pub help: Option<String>,
    #[serde(default, rename = "depends-on")]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub select: Vec<String>,
    #[serde(default)]
    pub range: Option<Vec<toml::Value>>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
}

/// A build profile.
#[derive(Debug, Default, Deserialize)]
pub struct ProfileConfig {
    pub target: Option<String>,
    pub inherits: Option<String>,
    #[serde(default, rename = "opt-level")]
    pub opt_level: Option<u32>,
    #[serde(default, rename = "debug-info")]
    pub debug_info: Option<bool>,
    #[serde(default)]
    pub lto: Option<String>,
    #[serde(default, rename = "boot-binary")]
    pub boot_binary: Option<String>,
    #[serde(default)]
    pub config: BTreeMap<String, toml::Value>,
    #[serde(default)]
    pub qemu: Option<ProfileQemu>,
    #[serde(default)]
    pub test: Option<ProfileTest>,
}

/// QEMU overrides in a profile.
#[derive(Debug, Default, Deserialize)]
pub struct ProfileQemu {
    pub memory: Option<u32>,
    pub cores: Option<u32>,
    #[serde(default, rename = "extra-args")]
    pub extra_args: Option<Vec<String>>,
}

/// Test overrides in a profile.
#[derive(Debug, Default, Deserialize)]
pub struct ProfileTest {
    pub timeout: Option<u32>,
}

/// `[qemu]` section.
#[derive(Debug, Deserialize)]
pub struct QemuConfig {
    #[serde(default = "default_machine")]
    pub machine: String,
    #[serde(default = "default_memory")]
    pub memory: u32,
    #[serde(default, rename = "extra-args")]
    pub extra_args: Vec<String>,
    #[serde(default)]
    pub test: QemuTestConfig,
}

impl Default for QemuConfig {
    fn default() -> Self {
        Self {
            machine: default_machine(),
            memory: default_memory(),
            extra_args: vec!["-serial".into(), "stdio".into()],
            test: QemuTestConfig::default(),
        }
    }
}

fn default_machine() -> String {
    "q35".into()
}
fn default_memory() -> u32 {
    256
}

/// `[qemu.test]` section.
#[derive(Debug, Deserialize)]
pub struct QemuTestConfig {
    #[serde(default = "default_success_exit")]
    #[serde(rename = "success-exit-code")]
    pub success_exit_code: u32,
    #[serde(default = "default_timeout")]
    pub timeout: u32,
    #[serde(default, rename = "extra-args")]
    pub extra_args: Vec<String>,
}

impl Default for QemuTestConfig {
    fn default() -> Self {
        Self {
            success_exit_code: default_success_exit(),
            timeout: default_timeout(),
            extra_args: Vec::new(),
        }
    }
}

fn default_success_exit() -> u32 {
    33
}
fn default_timeout() -> u32 {
    30
}

/// `[bootloader]` section.
#[derive(Debug, Deserialize)]
pub struct BootloaderConfig {
    #[serde(default = "default_bootloader_kind")]
    pub kind: String,
    #[serde(default, rename = "config-file")]
    pub config_file: Option<String>,
}

impl Default for BootloaderConfig {
    fn default() -> Self {
        Self {
            kind: default_bootloader_kind(),
            config_file: Some("limine.conf".into()),
        }
    }
}

fn default_bootloader_kind() -> String {
    "limine".into()
}

/// `[image]` section.
#[derive(Debug, Default, Deserialize)]
pub struct ImageConfig {
    #[serde(default, rename = "extra-files")]
    pub extra_files: BTreeMap<String, String>,
}

/// `[tests]` section.
#[derive(Debug, Default, Deserialize)]
pub struct TestsConfig {
    #[serde(default, rename = "host-testable")]
    pub host_testable: Vec<String>,
    #[serde(default, rename = "kernel-tests-dir")]
    pub kernel_tests_dir: Option<String>,
    #[serde(default)]
    pub crash: Vec<CrashTest>,
}

/// `[[tests.crash]]` entry.
#[derive(Debug, Deserialize)]
pub struct CrashTest {
    pub name: String,
    pub source: String,
    #[serde(rename = "expected-exit")]
    pub expected_exit: u32,
    #[serde(default, rename = "expect-output")]
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
    pub bootloader: BootloaderConfig,
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

/// Find the project root by looking for `hadron.toml`.
pub fn find_project_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("hadron.toml").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("could not find hadron.toml in any parent directory");
        }
    }
}

/// Load and resolve the full project configuration.
pub fn load_config(
    root: &Path,
    profile_name: Option<&str>,
    target_override: Option<&str>,
) -> Result<ResolvedConfig> {
    let toml_path = root.join("hadron.toml");
    let contents =
        std::fs::read_to_string(&toml_path).context("failed to read hadron.toml")?;
    let project_config: ProjectConfig =
        toml::from_str(&contents).context("failed to parse hadron.toml")?;

    let profile_name = profile_name.unwrap_or("default");

    // Collect merged config overrides from the profile inheritance chain.
    let profile_config_overrides =
        collect_profile_config(&project_config.profiles, profile_name)?;

    // Resolve profile with inheritance.
    let profile =
        resolve_profile(&project_config.profiles, profile_name, &project_config)?;

    // Target from override > profile > first in targets map.
    let target_name = target_override
        .map(String::from)
        .or_else(|| Some(profile.target.clone()))
        .unwrap();

    let target = project_config
        .targets
        .get(&target_name)
        .with_context(|| format!("target '{target_name}' not found in hadron.toml"))?;

    // Resolve config options with profile overrides.
    let options = resolve_options(
        &project_config.config.options,
        &profile_config_overrides,
    )?;

    // Apply select/depends-on validation.
    let options = apply_selects_and_validate(
        options,
        &project_config.config.options,
    )?;

    // Construct target (clone the relevant data since we consume the map).
    let resolved_target = TargetConfig {
        spec: target.spec.clone(),
        linker_script: target.linker_script.clone(),
    };

    Ok(ResolvedConfig {
        project: project_config.project,
        root: root.to_path_buf(),
        target_name,
        target: resolved_target,
        options,
        profile,
        qemu: project_config.qemu,
        bootloader: project_config.bootloader,
        image: project_config.image,
        tests: project_config.tests,
    })
}

/// Collect merged config overrides from a profile's inheritance chain.
/// Parent config is applied first, then child overrides on top.
fn collect_profile_config(
    profiles: &BTreeMap<String, ProfileConfig>,
    name: &str,
) -> Result<BTreeMap<String, toml::Value>> {
    let profile = profiles
        .get(name)
        .with_context(|| format!("profile '{name}' not found in hadron.toml"))?;

    let mut merged = if let Some(ref parent_name) = profile.inherits {
        collect_profile_config(profiles, parent_name)?
    } else {
        BTreeMap::new()
    };

    merged.extend(profile.config.clone());
    Ok(merged)
}

/// Resolve a profile by applying inheritance chain.
fn resolve_profile(
    profiles: &BTreeMap<String, ProfileConfig>,
    name: &str,
    project: &ProjectConfig,
) -> Result<ResolvedProfile> {
    let profile = profiles
        .get(name)
        .with_context(|| format!("profile '{name}' not found in hadron.toml"))?;

    // If this profile inherits from another, resolve the parent first.
    let parent = if let Some(ref parent_name) = profile.inherits {
        Some(resolve_profile(profiles, parent_name, project)?)
    } else {
        None
    };

    // Child overrides parent.
    let target = profile
        .target
        .clone()
        .or_else(|| parent.as_ref().map(|p| p.target.clone()))
        .or_else(|| {
            project
                .targets
                .keys()
                .next()
                .cloned()
        })
        .context("profile has no target and no targets defined")?;

    let opt_level = profile
        .opt_level
        .or(parent.as_ref().map(|p| p.opt_level))
        .unwrap_or(0);

    let debug_info = profile
        .debug_info
        .or(parent.as_ref().map(|p| p.debug_info))
        .unwrap_or(true);

    let lto = profile
        .lto
        .clone()
        .or_else(|| parent.as_ref().and_then(|p| p.lto.clone()));

    let boot_binary = profile
        .boot_binary
        .clone()
        .or_else(|| parent.as_ref().map(|p| p.boot_binary.clone()))
        .unwrap_or_else(|| "hadron-boot-limine".into());

    let qemu_memory = profile
        .qemu
        .as_ref()
        .and_then(|q| q.memory)
        .or(parent.as_ref().and_then(|p| p.qemu_memory));

    let qemu_cores = profile
        .qemu
        .as_ref()
        .and_then(|q| q.cores)
        .or(parent.as_ref().and_then(|p| p.qemu_cores));

    let qemu_extra_args = profile
        .qemu
        .as_ref()
        .and_then(|q| q.extra_args.clone())
        .or_else(|| parent.as_ref().and_then(|p| p.qemu_extra_args.clone()));

    let test_timeout = profile
        .test
        .as_ref()
        .and_then(|t| t.timeout)
        .or(parent.as_ref().and_then(|p| p.test_timeout));

    // Merge config: start with parent, override with child.
    let mut config = parent
        .as_ref()
        .map(|_| BTreeMap::new())
        .unwrap_or_default();
    // We don't have parent config values directly — they're already resolved into
    // options. Profile config overrides are applied during option resolution.
    config.extend(profile.config.clone());

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
    options: &BTreeMap<String, ConfigOption>,
    profile_overrides: &BTreeMap<String, toml::Value>,
) -> Result<BTreeMap<String, ResolvedValue>> {
    let mut resolved = BTreeMap::new();

    for (name, opt) in options {
        let value = profile_overrides
            .get(name)
            .unwrap_or(&opt.default);

        let resolved_value = match opt.ty.as_str() {
            "bool" => {
                let v = value
                    .as_bool()
                    .with_context(|| format!("option '{name}' expected bool"))?;
                ResolvedValue::Bool(v)
            }
            "u32" => {
                let v = parse_integer(value)
                    .with_context(|| format!("option '{name}' expected u32"))?;
                ResolvedValue::U32(v as u32)
            }
            "u64" => {
                let v = parse_integer(value)
                    .with_context(|| format!("option '{name}' expected u64"))?;
                ResolvedValue::U64(v)
            }
            "str" => {
                let v = value
                    .as_str()
                    .with_context(|| format!("option '{name}' expected string"))?;
                ResolvedValue::Str(v.to_string())
            }
            other => bail!("unknown config type '{other}' for option '{name}'"),
        };

        // Validate range.
        if let Some(ref range) = opt.range {
            ensure!(
                range.len() == 2,
                "option '{name}' range must have exactly 2 elements [min, max]"
            );
            let min = parse_integer(&range[0])
                .context("range min")?;
            let max = parse_integer(&range[1])
                .context("range max")?;
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
    defs: &BTreeMap<String, ConfigOption>,
) -> Result<BTreeMap<String, ResolvedValue>> {
    // Apply selects: if option X is enabled and selects Y, enable Y.
    // Iterate until stable (handles transitive selects).
    loop {
        let mut changed = false;
        for (name, def) in defs {
            let is_enabled = matches!(options.get(name), Some(ResolvedValue::Bool(true)));
            if is_enabled {
                for selected in &def.select {
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

    // Validate depends-on: if X is enabled, all its dependencies must be enabled.
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
                        // Non-bool deps: just check they exist.
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

/// Parse an integer from a TOML value (supports both integer and hex-string formats).
fn parse_integer(value: &toml::Value) -> Result<u64> {
    match value {
        toml::Value::Integer(i) => Ok(*i as u64),
        toml::Value::String(s) => {
            let s = s.replace('_', "");
            if let Some(hex) = s.strip_prefix("0x") {
                u64::from_str_radix(hex, 16)
                    .context("invalid hex integer")
            } else {
                s.parse::<u64>().context("invalid integer")
            }
        }
        _ => bail!("expected integer or hex-string, got {value:?}"),
    }
}

/// Print resolved config to stdout for debugging.
pub fn print_resolved(config: &ResolvedConfig) {
    println!("Project: {} v{}", config.project.name, config.project.version);
    println!("Target: {}", config.target_name);
    println!("  spec: {}", config.target.spec);
    if let Some(ref ld) = config.target.linker_script {
        println!("  linker-script: {ld}");
    }
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

    #[test]
    fn parse_hex_string() {
        let val = toml::Value::String("0x10_0000".into());
        assert_eq!(parse_integer(&val).unwrap(), 0x10_0000);
    }

    #[test]
    fn parse_plain_integer() {
        let val = toml::Value::Integer(42);
        assert_eq!(parse_integer(&val).unwrap(), 42);
    }

    #[test]
    fn select_enables_dependency() {
        let mut options = BTreeMap::new();
        options.insert("smp".into(), ResolvedValue::Bool(true));
        options.insert("apic".into(), ResolvedValue::Bool(false));
        options.insert("acpi".into(), ResolvedValue::Bool(true));

        let mut defs = BTreeMap::new();
        defs.insert("smp".into(), ConfigOption {
            ty: "bool".into(),
            default: toml::Value::Boolean(false),
            help: None,
            depends_on: vec!["acpi".into()],
            select: vec!["apic".into()],
            range: None,
            choices: None,
        });
        defs.insert("apic".into(), ConfigOption {
            ty: "bool".into(),
            default: toml::Value::Boolean(false),
            help: None,
            depends_on: vec![],
            select: vec![],
            range: None,
            choices: None,
        });
        defs.insert("acpi".into(), ConfigOption {
            ty: "bool".into(),
            default: toml::Value::Boolean(true),
            help: None,
            depends_on: vec![],
            select: vec![],
            range: None,
            choices: None,
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
        defs.insert("smp".into(), ConfigOption {
            ty: "bool".into(),
            default: toml::Value::Boolean(false),
            help: None,
            depends_on: vec!["acpi".into()],
            select: vec![],
            range: None,
            choices: None,
        });
        defs.insert("acpi".into(), ConfigOption {
            ty: "bool".into(),
            default: toml::Value::Boolean(true),
            help: None,
            depends_on: vec![],
            select: vec![],
            range: None,
            choices: None,
        });

        let result = apply_selects_and_validate(options, &defs);
        assert!(result.is_err());
    }
}
