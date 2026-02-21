//! Crate compilation via direct `rustc` invocation.
//!
//! Assembles rustc flags for each crate based on its resolved definition,
//! invokes rustc, and tracks output artifacts for downstream extern linking.

use crate::config::{ResolvedConfig, ResolvedValue};
use crate::crate_graph::ResolvedCrate;
use crate::model::CrateType;
use crate::rustc_cmd::RustcCommandBuilder;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Controls how a crate is compiled.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompileMode {
    /// Full compilation: emit dep-info, metadata, and link artifacts.
    Build,
    /// Type-check only: emit metadata (no linking).
    Check,
    /// Lint with clippy-driver: emit metadata (no linking).
    Clippy,
}

/// Tracks compiled artifacts so downstream crates can find their --extern paths.
#[derive(Default)]
pub struct ArtifactMap {
    /// Maps crate name -> path to rlib/dylib.
    artifacts: HashMap<String, PathBuf>,
}

impl ArtifactMap {
    pub fn insert(&mut self, name: &str, path: PathBuf) {
        self.artifacts.insert(name.to_string(), path);
    }

    pub fn get(&self, name: &str) -> Option<&Path> {
        self.artifacts.get(name).map(|p| p.as_path())
    }

}

/// Generate the `hadron_config` crate source and compile it.
///
/// Only emits constants for options with `Binding::Const`. Options without
/// any `binding const` line are not included. Build metadata (TARGET,
/// PROFILE, VERSION) is always emitted.
pub fn build_config_crate(
    config: &ResolvedConfig,
    target_spec: &str,
    sysroot_dir: &Path,
) -> Result<PathBuf> {
    use crate::model::Binding;

    let gen_dir = config.root.join("build/generated");
    std::fs::create_dir_all(&gen_dir)?;

    // Generate hadron_config.rs.
    let mut source = String::new();
    source.push_str("//! Auto-generated kernel configuration constants.\n");
    source.push_str("#![no_std]\n\n");

    // Helper: check if an option should emit a constant.
    // Options with no bindings at all (legacy Rhai-defined) emit constants for
    // backwards compatibility. Options with explicit bindings only emit if
    // `Binding::Const` is present.
    let should_emit_const = |name: &str| -> bool {
        match config.bindings.get(name) {
            None => true,  // no bindings = legacy behavior, emit everything
            Some(bs) => bs.contains(&Binding::Const),
        }
    };

    // Collect dotted keys (group sub-fields) by their prefix for nested module codegen.
    let mut group_fields: BTreeMap<String, Vec<(String, &ResolvedValue)>> = BTreeMap::new();

    for (name, value) in &config.options {
        if let Some(dot_pos) = name.find('.') {
            let prefix = &name[..dot_pos];
            let field = &name[dot_pos + 1..];
            if should_emit_const(name) {
                group_fields
                    .entry(prefix.to_string())
                    .or_default()
                    .push((field.to_string(), value));
            }
            continue;
        }

        if !should_emit_const(name) {
            continue;
        }

        emit_const(&mut source, &name.to_uppercase(), value, "");
    }

    // Generate nested modules for group sub-fields.
    for (prefix, fields) in &group_fields {
        source.push_str(&format!("pub mod {} {{\n", prefix.to_lowercase()));
        for (field, value) in fields {
            emit_const(&mut source, &field.to_uppercase(), value, "    ");
        }
        source.push_str("}\n");
    }

    // Always emit build metadata.
    source.push_str(&format!(
        "\npub const TARGET: &str = \"{}\";\n",
        config.target_name.replace('\\', "\\\\").replace('"', "\\\"")
    ));
    source.push_str(&format!(
        "pub const PROFILE: &str = \"{}\";\n",
        config.profile.name.replace('\\', "\\\\").replace('"', "\\\"")
    ));
    source.push_str(&format!(
        "pub const VERSION: &str = \"{}\";\n",
        config.project.version.replace('\\', "\\\\").replace('"', "\\\"")
    ));

    let src_path = gen_dir.join("hadron_config.rs");
    std::fs::write(&src_path, &source)?;

    // Compile it.
    let out_dir = config
        .root
        .join("build/kernel")
        .join(&config.target_name)
        .join("debug");
    std::fs::create_dir_all(&out_dir)?;

    let mut cmd = RustcCommandBuilder::new("rustc");
    cmd.crate_name("hadron_config")
        .edition("2024")
        .crate_type("rlib")
        .unstable_options()
        .panic_abort()
        .opt_level(config.profile.opt_level)
        .target(target_spec)
        .sysroot(sysroot_dir)
        .out_dir(&out_dir)
        .emit("dep-info,metadata,link")
        .source(&src_path);

    cmd.run_checked("compile")?;

    let rlib = out_dir.join("libhadron_config.rlib");
    if !rlib.exists() {
        bail!("expected libhadron_config.rlib not found");
    }

    Ok(rlib)
}

/// Emit a single `pub const` line for a resolved config value.
fn emit_const(source: &mut String, name: &str, value: &ResolvedValue, indent: &str) {
    match value {
        ResolvedValue::Bool(v) => {
            source.push_str(&format!("{indent}pub const {name}: bool = {v};\n"));
        }
        ResolvedValue::U32(v) => {
            source.push_str(&format!("{indent}pub const {name}: u32 = {v};\n"));
        }
        ResolvedValue::U64(v) => {
            source.push_str(&format!("{indent}pub const {name}: u64 = {v:#x};\n"));
        }
        ResolvedValue::Str(v) | ResolvedValue::Choice(v) => {
            source.push_str(&format!(
                "{indent}pub const {name}: &str = \"{}\";\n",
                v.replace('\\', "\\\\").replace('"', "\\\"")
            ));
        }
        ResolvedValue::List(v) => {
            let quoted: Vec<String> = v
                .iter()
                .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
                .collect();
            source.push_str(&format!(
                "{indent}pub const {name}: &[&str] = &[{}];\n",
                quoted.join(", ")
            ));
        }
    }
}

/// Compile a single crate.
///
/// Dispatches between host and cross-compilation based on `krate.target`.
/// Host crates (target == "host") are compiled for the host triple without a
/// custom sysroot. Cross crates use the provided `target_spec` and `sysroot_dir`.
/// The `config_rlib` is only linked for crates in config-enabled groups.
/// The `mode` controls full build, check-only, or clippy lint.
pub fn compile_crate(
    krate: &ResolvedCrate,
    config: &ResolvedConfig,
    target_spec: Option<&str>,
    sysroot_dir: Option<&Path>,
    artifacts: &ArtifactMap,
    config_rlib: Option<&Path>,
    out_dir_suffix: Option<&str>,
    mode: CompileMode,
) -> Result<PathBuf> {
    let is_host = krate.target == "host";

    if is_host {
        compile_crate_host(krate, &config.root, artifacts)
    } else {
        compile_crate_cross(
            krate,
            config,
            target_spec.expect("target_spec required for cross compilation"),
            sysroot_dir.expect("sysroot_dir required for cross compilation"),
            artifacts,
            config_rlib,
            out_dir_suffix,
            mode,
            &[],
        )
    }
}

/// Re-link a binary crate with additional object files (e.g., HKIF blob).
///
/// This performs a full cross-compilation of the crate, appending the given
/// object files as extra linker arguments. Used for the two-pass HKIF link.
pub fn relink_with_objects(
    krate: &ResolvedCrate,
    config: &ResolvedConfig,
    target_spec: &str,
    sysroot_dir: &Path,
    artifacts: &ArtifactMap,
    config_rlib: Option<&Path>,
    extra_objects: &[PathBuf],
) -> Result<PathBuf> {
    compile_crate_cross(
        krate,
        config,
        target_spec,
        sysroot_dir,
        artifacts,
        config_rlib,
        None,
        CompileMode::Build,
        extra_objects,
    )
}

/// Compile a crate for a custom (cross) target.
fn compile_crate_cross(
    krate: &ResolvedCrate,
    config: &ResolvedConfig,
    target_spec: &str,
    sysroot_dir: &Path,
    artifacts: &ArtifactMap,
    config_rlib: Option<&Path>,
    out_dir_suffix: Option<&str>,
    mode: CompileMode,
    extra_link_objects: &[PathBuf],
) -> Result<PathBuf> {
    let suffix = out_dir_suffix.unwrap_or(&krate.target);
    let out_dir = config
        .root
        .join("build/kernel")
        .join(suffix)
        .join("debug");
    std::fs::create_dir_all(&out_dir)?;

    let is_check = mode == CompileMode::Check || mode == CompileMode::Clippy;

    let crate_type = match krate.crate_type {
        CrateType::ProcMacro => "proc-macro",
        CrateType::Bin => if is_check { "lib" } else { "bin" },
        CrateType::Lib => "rlib",
    };

    // Use clippy-driver for Clippy mode on project crates.
    let binary = if mode == CompileMode::Clippy && krate.is_project_crate {
        "clippy-driver"
    } else {
        "rustc"
    };

    let mut cmd = RustcCommandBuilder::new(binary);
    cmd.crate_name(&crate_name_sanitized(&krate.name))
        .edition(&krate.edition)
        .crate_type(crate_type)
        .unstable_options()
        .panic_abort()
        .opt_level(config.profile.opt_level)
        .force_frame_pointers();

    if config.profile.debug_info {
        cmd.debug_info(2);
    }

    // Clippy lint flags for project crates.
    if mode == CompileMode::Clippy && krate.is_project_crate {
        cmd.warn("clippy::all").warn("clippy::pedantic");
    }

    // Target and sysroot.
    cmd.target(target_spec).sysroot(sysroot_dir);

    // Search paths for transitive deps and host proc-macros.
    cmd.search_path(&out_dir)
        .search_path(&config.root.join("build/host"));

    // Linker args for binary crates (only in Build mode).
    if !is_check && krate.crate_type == CrateType::Bin {
        if let Some(ref ld_script) = krate.linker_script {
            let ld_path = config.root.join(ld_script);
            cmd.link_arg(&format!("-T{}", ld_path.display()));
        }
        cmd.link_arg("--gc-sections");

        // Extra object files (e.g., HKIF blob for pass-2 link).
        for obj in extra_link_objects {
            cmd.link_arg(&obj.display().to_string());
        }
    }

    // Features as --cfg.
    for feat in &krate.features {
        cmd.feature(feat);
    }

    // Config cfgs for options with Binding::Cfg or Binding::CfgCumulative.
    if config_rlib.is_some() {
        use crate::model::Binding;

        for (name, value) in &config.options {
            let opt_bindings = config.bindings.get(name);

            // Legacy behavior: options with no bindings emit cfg for Bool(true).
            let has_cfg = opt_bindings.map_or(false, |bs| bs.contains(&Binding::Cfg));
            let has_cfg_cumulative = opt_bindings.map_or(false, |bs| bs.contains(&Binding::CfgCumulative));
            let is_legacy = opt_bindings.is_none();

            if has_cfg {
                match value {
                    ResolvedValue::Bool(true) => {
                        cmd.cfg(&format!("hadron_{name}"));
                    }
                    ResolvedValue::Choice(v) | ResolvedValue::Str(v) => {
                        cmd.cfg(&format!("hadron_{name}=\"{v}\""));
                    }
                    _ => {}
                }
            } else if has_cfg_cumulative {
                // Emit cfg for all choice values up to and including the selected one.
                if let ResolvedValue::Choice(selected) = value {
                    cmd.cfg(&format!("hadron_{name}=\"{selected}\""));

                    // Use the choice variants from the config definition for ordering.
                    if let Some(variants) = config.choices.get(name) {
                        if let Some(selected_idx) = variants.iter().position(|v| v == selected) {
                            for variant in &variants[..=selected_idx] {
                                cmd.cfg(&format!("hadron_{name}_{variant}"));
                            }
                        }
                    }
                }
            } else if is_legacy {
                // Backwards compatibility: emit hadron_<name> for Bool(true).
                if let ResolvedValue::Bool(true) = value {
                    cmd.cfg(&format!("hadron_{name}"));
                }
            }
        }
    }

    // Extern deps.
    for dep in &krate.deps {
        if let Some(path) = artifacts.get(&dep.crate_name) {
            cmd.add_extern(&dep.extern_name, path);
        }
    }

    // Link config crate if provided.
    if let Some(config_path) = config_rlib {
        cmd.add_extern("hadron_config", config_path);
    }

    // Incremental compilation.
    let inc_dir = config
        .root
        .join("build/incremental")
        .join(crate_name_sanitized(&krate.name));
    std::fs::create_dir_all(&inc_dir)?;
    cmd.incremental(&inc_dir);

    // Output.
    cmd.out_dir(&out_dir);
    if is_check {
        cmd.emit("dep-info,metadata");
    } else if crate_type == "bin" {
        cmd.emit("dep-info,link");
    } else {
        cmd.emit("dep-info,metadata,link");
    }

    // Source file.
    cmd.source(&krate.root_file);

    let verb = if mode == CompileMode::Clippy { "lint" } else { "compile" };
    cmd.run_checked(verb)?;

    // Determine output artifact path.
    let artifact = if !is_check && krate.crate_type == CrateType::Bin {
        out_dir.join(crate_name_sanitized(&krate.name))
    } else if is_check {
        out_dir.join(format!(
            "lib{}.rmeta",
            crate_name_sanitized(&krate.name)
        ))
    } else {
        out_dir.join(format!(
            "lib{}.rlib",
            crate_name_sanitized(&krate.name)
        ))
    };

    Ok(artifact)
}

/// Compile a crate for the host triple (proc-macros and their deps).
fn compile_crate_host(
    krate: &ResolvedCrate,
    project_root: &Path,
    artifacts: &ArtifactMap,
) -> Result<PathBuf> {
    let out_dir = project_root.join("build/host");
    std::fs::create_dir_all(&out_dir)?;

    // Locate host sysroot lib dir for proc_macro and std.
    let host_sysroot_lib = host_sysroot_lib_dir()?;

    let crate_type = match krate.crate_type {
        CrateType::ProcMacro => "proc-macro",
        _ => "lib",
    };

    let mut cmd = RustcCommandBuilder::new("rustc");
    cmd.crate_name(&crate_name_sanitized(&krate.name))
        .edition(&krate.edition)
        .crate_type(crate_type);

    // Add search paths for transitive deps and host sysroot (proc_macro, std).
    cmd.search_path(&out_dir).search_path(&host_sysroot_lib);

    // For proc-macro crates, inject the compiler's proc_macro crate via
    // `--extern proc_macro` (no path) + `-C prefer-dynamic`. This is how
    // cargo provides the proc_macro bridge to proc-macro crates.
    if crate_type == "proc-macro" {
        cmd.prefer_dynamic().add_extern_no_path("proc_macro");
    }

    // Per-crate cfg flags (e.g. proc-macro2 needs `wrap_proc_macro`).
    for flag in &krate.cfg_flags {
        cmd.cfg(flag);
    }

    // Features.
    for feat in &krate.features {
        cmd.feature(feat);
    }

    // Extern deps.
    for dep in &krate.deps {
        if let Some(path) = artifacts.get(&dep.crate_name) {
            cmd.add_extern(&dep.extern_name, path);
        }
    }

    cmd.out_dir(&out_dir)
        .emit("dep-info,metadata,link")
        .source(&krate.root_file);

    cmd.run_checked("compile")?;

    // Determine artifact path. For proc-macros it's a dylib.
    let artifact = if crate_type == "proc-macro" {
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };
        out_dir.join(format!(
            "lib{}.{ext}",
            crate_name_sanitized(&krate.name)
        ))
    } else {
        out_dir.join(format!(
            "lib{}.rlib",
            crate_name_sanitized(&krate.name)
        ))
    };

    if !artifact.exists() {
        bail!(
            "expected artifact not found for host crate '{}': {}",
            krate.name,
            artifact.display()
        );
    }

    Ok(artifact)
}

/// Locate the host triple's sysroot library directory.
fn host_sysroot_lib_dir() -> Result<PathBuf> {
    let sysroot_output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .context("failed to run rustc --print sysroot")?;
    let sysroot = String::from_utf8(sysroot_output.stdout)?
        .trim()
        .to_string();

    let host_output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("failed to run rustc -vV")?;
    let version_info = String::from_utf8(host_output.stdout)?;
    let host = version_info
        .lines()
        .find(|l| l.starts_with("host:"))
        .and_then(|l| l.strip_prefix("host: "))
        .context("could not determine host triple")?
        .to_string();

    Ok(PathBuf::from(sysroot)
        .join("lib/rustlib")
        .join(host)
        .join("lib"))
}

/// Sanitize a crate name for use as a rustc crate name (hyphens -> underscores).
pub fn crate_name_sanitized(name: &str) -> String {
    name.replace('-', "_")
}

/// Compute the output directory for a crate compilation.
///
/// Host crates go to `build/host/`, cross crates to `build/kernel/<target>/debug/`.
pub fn crate_out_dir(krate: &ResolvedCrate, project_root: &Path, out_dir_suffix: Option<&str>) -> PathBuf {
    if krate.target == "host" {
        project_root.join("build/host")
    } else {
        let suffix = out_dir_suffix.unwrap_or(&krate.target);
        project_root
            .join("build/kernel")
            .join(suffix)
            .join("debug")
    }
}

/// Predict the artifact path for a crate without compiling.
pub fn crate_artifact_path(
    krate: &ResolvedCrate,
    project_root: &Path,
    out_dir_suffix: Option<&str>,
    mode: CompileMode,
) -> PathBuf {
    let out_dir = crate_out_dir(krate, project_root, out_dir_suffix);
    let is_check = mode == CompileMode::Check || mode == CompileMode::Clippy;
    let sanitized = crate_name_sanitized(&krate.name);

    if krate.target == "host" {
        // Host crates: proc-macros are dylibs, others are rlibs.
        if krate.crate_type == CrateType::ProcMacro {
            let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
            out_dir.join(format!("lib{sanitized}.{ext}"))
        } else {
            out_dir.join(format!("lib{sanitized}.rlib"))
        }
    } else if !is_check && krate.crate_type == CrateType::Bin {
        out_dir.join(&sanitized)
    } else if is_check {
        out_dir.join(format!("lib{sanitized}.rmeta"))
    } else {
        out_dir.join(format!("lib{sanitized}.rlib"))
    }
}

/// Predict the dep-info path for a crate.
pub fn crate_dep_info_path(
    krate: &ResolvedCrate,
    project_root: &Path,
    out_dir_suffix: Option<&str>,
) -> PathBuf {
    let out_dir = crate_out_dir(krate, project_root, out_dir_suffix);
    out_dir.join(format!("{}.d", crate_name_sanitized(&krate.name)))
}

/// Predict the dep-info path for the hadron_config crate.
pub fn config_crate_dep_info_path(config: &ResolvedConfig) -> PathBuf {
    let out_dir = config
        .root
        .join("build/kernel")
        .join(&config.target_name)
        .join("debug");
    out_dir.join("hadron_config.d")
}

/// Hash an arbitrary list of `OsStr` values for cache keying.
pub fn hash_args(args: &[&OsStr]) -> String {
    let mut hasher = Sha256::new();
    for arg in args {
        hasher.update(arg.as_encoded_bytes());
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}
