//! Crate compilation via direct `rustc` invocation.
//!
//! Assembles rustc flags for each crate based on its resolved definition,
//! invokes rustc, and tracks output artifacts for downstream extern linking.

use crate::config::{ResolvedConfig, ResolvedValue};
use crate::crate_graph::ResolvedCrate;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
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
pub fn build_config_crate(
    config: &ResolvedConfig,
    target_spec: &str,
    sysroot_dir: &Path,
) -> Result<PathBuf> {
    let gen_dir = config.root.join("build/generated");
    std::fs::create_dir_all(&gen_dir)?;

    // Generate hadron_config.rs.
    let mut source = String::new();
    source.push_str("//! Auto-generated kernel configuration constants.\n");
    source.push_str("#![no_std]\n\n");

    for (name, value) in &config.options {
        match value {
            ResolvedValue::Bool(v) => {
                source.push_str(&format!(
                    "pub const {}: bool = {v};\n",
                    name.to_uppercase()
                ));
            }
            ResolvedValue::U32(v) => {
                source.push_str(&format!(
                    "pub const {}: u32 = {v};\n",
                    name.to_uppercase()
                ));
            }
            ResolvedValue::U64(v) => {
                source.push_str(&format!(
                    "pub const {}: u64 = {v:#x};\n",
                    name.to_uppercase()
                ));
            }
            ResolvedValue::Str(v) => {
                source.push_str(&format!(
                    "pub const {}: &str = \"{v}\";\n",
                    name.to_uppercase()
                ));
            }
        }
    }

    let src_path = gen_dir.join("hadron_config.rs");
    std::fs::write(&src_path, &source)?;

    // Compile it.
    let out_dir = config
        .root
        .join("build/kernel")
        .join(&config.target_name)
        .join("debug");
    std::fs::create_dir_all(&out_dir)?;

    let mut cmd = Command::new("rustc");
    cmd.arg("--crate-name")
        .arg("hadron_config")
        .arg("--edition=2024")
        .arg("--crate-type=rlib")
        .arg("-Zunstable-options")
        .arg("-Cpanic=abort")
        .arg(format!("-Copt-level={}", config.profile.opt_level))
        .arg("--target")
        .arg(target_spec)
        .arg("--sysroot")
        .arg(sysroot_dir)
        .arg("--out-dir")
        .arg(&out_dir)
        .arg("--emit=dep-info,metadata,link")
        .arg(&src_path);

    let output = cmd.output().context("failed to run rustc for hadron_config")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to compile hadron_config:\n{stderr}");
    }

    let rlib = out_dir.join("libhadron_config.rlib");
    if !rlib.exists() {
        bail!("expected libhadron_config.rlib not found");
    }

    Ok(rlib)
}

/// Compile a single crate for a custom target.
///
/// The `out_dir_suffix` determines the output subdirectory under `build/kernel/`.
/// The `linker_script` is only applied to binary crates in `Build` mode.
/// The `mode` controls whether to do a full build, check-only, or clippy lint.
pub fn compile_crate(
    krate: &ResolvedCrate,
    config: &ResolvedConfig,
    target_spec: &str,
    sysroot_dir: &Path,
    artifacts: &ArtifactMap,
    config_rlib: Option<&Path>,
    out_dir_suffix: Option<&str>,
    linker_script: Option<&Path>,
    mode: CompileMode,
) -> Result<PathBuf> {
    let suffix = out_dir_suffix.unwrap_or(&config.target_name);
    let out_dir = config
        .root
        .join("build/kernel")
        .join(suffix)
        .join("debug");
    std::fs::create_dir_all(&out_dir)?;

    let is_check = mode == CompileMode::Check || mode == CompileMode::Clippy;

    let crate_type = if krate.crate_type == "proc-macro" {
        "proc-macro"
    } else if krate.crate_type == "bin" {
        if is_check { "lib" } else { "bin" }
    } else {
        "rlib"
    };

    // Use clippy-driver for Clippy mode on project crates.
    let binary = if mode == CompileMode::Clippy && is_project_crate(krate) {
        "clippy-driver"
    } else {
        "rustc"
    };

    let mut cmd = Command::new(binary);
    cmd.arg("--crate-name")
        .arg(crate_name_sanitized(&krate.name))
        .arg(format!("--edition={}", krate.edition))
        .arg(format!("--crate-type={crate_type}"))
        .arg("-Zunstable-options")
        .arg("-Cpanic=abort")
        .arg(format!("-Copt-level={}", config.profile.opt_level))
        .arg("-Cforce-frame-pointers=yes");

    if config.profile.debug_info {
        cmd.arg("-Cdebuginfo=2");
    }

    // Clippy lint flags for project crates.
    if mode == CompileMode::Clippy && is_project_crate(krate) {
        cmd.arg("-Wclippy::all").arg("-Wclippy::pedantic");
    }

    // Target and sysroot.
    cmd.arg("--target")
        .arg(target_spec)
        .arg("--sysroot")
        .arg(sysroot_dir);

    // Search paths for transitive kernel deps and host proc-macros.
    cmd.arg("-L").arg(&out_dir);
    cmd.arg("-L").arg(config.root.join("build/host"));

    // Linker args for binary crates (only in Build mode).
    if !is_check && krate.crate_type == "bin" {
        if let Some(ld_script) = linker_script {
            cmd.arg(format!("-Clink-arg=-T{}", ld_script.display()));
        }
        cmd.arg("-Clink-arg=--gc-sections");
    }

    // Features as --cfg.
    for feat in &krate.features {
        cmd.arg("--cfg").arg(format!("feature=\"{feat}\""));
    }

    // Config cfgs for bool options.
    for (name, value) in &config.options {
        if let ResolvedValue::Bool(true) = value {
            cmd.arg("--cfg").arg(format!("hadron_{name}"));
        }
    }

    // Extern deps.
    for dep in &krate.deps {
        if dep.is_proc_macro {
            // Proc-macro extern: look in host artifacts.
            if let Some(path) = artifacts.get(&dep.crate_name) {
                cmd.arg("--extern")
                    .arg(format!("{}={}", dep.extern_name, path.display()));
            }
        } else if let Some(path) = artifacts.get(&dep.crate_name) {
            cmd.arg("--extern")
                .arg(format!("{}={}", dep.extern_name, path.display()));
        }
    }

    // Always link hadron_config for kernel crates.
    if let Some(config_path) = config_rlib {
        cmd.arg("--extern")
            .arg(format!("hadron_config={}", config_path.display()));
    }

    // Incremental compilation.
    let inc_dir = config
        .root
        .join("build/incremental")
        .join(crate_name_sanitized(&krate.name));
    std::fs::create_dir_all(&inc_dir)?;
    cmd.arg(format!("-Cincremental={}", inc_dir.display()));

    // Output.
    cmd.arg("--out-dir").arg(&out_dir);
    if is_check {
        cmd.arg("--emit=dep-info,metadata");
    } else if crate_type == "bin" {
        cmd.arg("--emit=dep-info,link");
    } else {
        cmd.arg("--emit=dep-info,metadata,link");
    }

    // Source file.
    cmd.arg(&krate.root_file);

    let output = cmd
        .output()
        .with_context(|| format!("failed to run {binary} for {}", krate.name))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let verb = if mode == CompileMode::Clippy { "lint" } else { "compile" };
        bail!("failed to {verb} '{}':\n{stderr}", krate.name);
    }

    // Determine output artifact path.
    let artifact = if !is_check && krate.crate_type == "bin" {
        out_dir.join(crate_name_sanitized(&krate.name))
    } else if is_check {
        // Check/clippy: metadata file (.rmeta).
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

/// Whether this crate is a project crate (not vendored or sysroot).
fn is_project_crate(krate: &ResolvedCrate) -> bool {
    let path = krate.path.to_string_lossy();
    path.contains("/kernel/") || path.contains("/crates/") || path.contains("/userspace/")
}

/// Compile a single crate for the host triple (proc-macros and their deps).
pub fn compile_host_crate(
    krate: &ResolvedCrate,
    project_root: &Path,
    artifacts: &ArtifactMap,
) -> Result<PathBuf> {
    let out_dir = project_root.join("build/host");
    std::fs::create_dir_all(&out_dir)?;

    // Locate host sysroot lib dir for proc_macro and std.
    let host_sysroot_lib = host_sysroot_lib_dir()?;

    let crate_type = if krate.crate_type == "proc-macro" {
        "proc-macro"
    } else {
        "lib"
    };

    let mut cmd = Command::new("rustc");
    cmd.arg("--crate-name")
        .arg(crate_name_sanitized(&krate.name))
        .arg(format!("--edition={}", krate.edition))
        .arg(format!("--crate-type={crate_type}"));

    // Add search paths for transitive deps and host sysroot (proc_macro, std).
    cmd.arg("-L").arg(&out_dir);
    cmd.arg("-L").arg(&host_sysroot_lib);

    // For proc-macro crates, inject the compiler's proc_macro crate via
    // `--extern proc_macro` (no path) + `-C prefer-dynamic`. This is how
    // cargo provides the proc_macro bridge to proc-macro crates.
    if crate_type == "proc-macro" {
        cmd.arg("-C").arg("prefer-dynamic");
        cmd.arg("--extern").arg("proc_macro");
    }

    // proc-macro2 requires build.rs cfg flags we must set manually.
    if krate.name == "proc-macro2" {
        cmd.arg("--cfg").arg("wrap_proc_macro");
    }

    // Features.
    for feat in &krate.features {
        cmd.arg("--cfg").arg(format!("feature=\"{feat}\""));
    }

    // Extern deps.
    for dep in &krate.deps {
        if let Some(path) = artifacts.get(&dep.crate_name) {
            cmd.arg("--extern")
                .arg(format!("{}={}", dep.extern_name, path.display()));
        }
    }

    cmd.arg("--out-dir").arg(&out_dir);
    cmd.arg("--emit=dep-info,metadata,link");
    cmd.arg(&krate.root_file);

    let output = cmd
        .output()
        .with_context(|| format!("failed to run rustc for host crate {}", krate.name))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to compile host crate '{}':\n{stderr}", krate.name);
    }

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

/// Compute the output directory for a kernel crate compilation.
pub fn kernel_out_dir(config: &ResolvedConfig, out_dir_suffix: Option<&str>) -> PathBuf {
    let suffix = out_dir_suffix.unwrap_or(&config.target_name);
    config
        .root
        .join("build/kernel")
        .join(suffix)
        .join("debug")
}

/// Predict the artifact path for a kernel crate without compiling.
pub fn crate_artifact_path(
    krate: &ResolvedCrate,
    config: &ResolvedConfig,
    out_dir_suffix: Option<&str>,
    mode: CompileMode,
) -> PathBuf {
    let out_dir = kernel_out_dir(config, out_dir_suffix);
    let is_check = mode == CompileMode::Check || mode == CompileMode::Clippy;
    let sanitized = crate_name_sanitized(&krate.name);

    if !is_check && krate.crate_type == "bin" {
        out_dir.join(&sanitized)
    } else if is_check {
        out_dir.join(format!("lib{sanitized}.rmeta"))
    } else {
        out_dir.join(format!("lib{sanitized}.rlib"))
    }
}

/// Predict the dep-info path for a kernel crate.
pub fn crate_dep_info_path(
    krate: &ResolvedCrate,
    config: &ResolvedConfig,
    out_dir_suffix: Option<&str>,
) -> PathBuf {
    let out_dir = kernel_out_dir(config, out_dir_suffix);
    out_dir.join(format!("{}.d", crate_name_sanitized(&krate.name)))
}

/// Predict the artifact path for a host crate without compiling.
pub fn host_crate_artifact_path(krate: &ResolvedCrate, project_root: &Path) -> PathBuf {
    let out_dir = project_root.join("build/host");
    let sanitized = crate_name_sanitized(&krate.name);

    if krate.crate_type == "proc-macro" {
        let ext = if cfg!(target_os = "macos") {
            "dylib"
        } else {
            "so"
        };
        out_dir.join(format!("lib{sanitized}.{ext}"))
    } else {
        out_dir.join(format!("lib{sanitized}.rlib"))
    }
}

/// Predict the dep-info path for a host crate.
pub fn host_crate_dep_info_path(krate: &ResolvedCrate, project_root: &Path) -> PathBuf {
    let out_dir = project_root.join("build/host");
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
