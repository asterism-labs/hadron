//! rust-project.json generation for rust-analyzer.
//!
//! Walks the resolved crate graph and emits a `rust-project.json` file
//! at the project root so rust-analyzer can provide IDE features for
//! non-Cargo projects.

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::compile;
use crate::config::{ResolvedConfig, ResolvedValue};
use crate::crate_graph::{self, CrateContext, CrateRegistry, ResolvedCrate};
use crate::sysroot;

/// Top-level rust-project.json structure.
#[derive(Serialize)]
struct RustProject {
    sysroot_src: PathBuf,
    crates: Vec<CrateEntry>,
}

/// A single crate entry in the rust-project.json.
#[derive(Serialize)]
struct CrateEntry {
    display_name: String,
    root_module: PathBuf,
    edition: String,
    deps: Vec<DepEntry>,
    cfg: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target: Option<String>,
    is_proc_macro: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    proc_macro_dylib_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    is_workspace_member: bool,
}

/// A dependency reference (by crate index).
#[derive(Serialize)]
struct DepEntry {
    #[serde(rename = "crate")]
    crate_idx: usize,
    name: String,
}

/// Generate `rust-project.json` at the project root.
pub fn generate_rust_project(config: &ResolvedConfig) -> Result<PathBuf> {
    let sysroot_src = sysroot::sysroot_src_dir()?;
    let registry = crate_graph::load_crate_registry(&config.root)?;

    // Resolve all contexts.
    let host_crates =
        crate_graph::resolve_and_sort(&registry, &config.root, &sysroot_src, &CrateContext::Host)?;
    let kernel_crates = crate_graph::resolve_and_sort(
        &registry,
        &config.root,
        &sysroot_src,
        &CrateContext::Kernel,
    )?;

    // Build a combined crate list. Sysroot crates are handled by sysroot_src.
    // Order: host crates first (proc-macros), then kernel crates.
    let mut all_crates: Vec<&ResolvedCrate> = Vec::new();
    let mut name_to_idx: BTreeMap<String, usize> = BTreeMap::new();

    // Add host crates.
    for krate in &host_crates {
        let idx = all_crates.len();
        name_to_idx.insert(krate.name.clone(), idx);
        all_crates.push(krate);
    }

    // Add kernel crates (skip if already present from host).
    for krate in &kernel_crates {
        if !name_to_idx.contains_key(&krate.name) {
            let idx = all_crates.len();
            name_to_idx.insert(krate.name.clone(), idx);
            all_crates.push(krate);
        }
    }

    // Build cfg flags from resolved config options.
    let config_cfgs = build_config_cfgs(config);

    // Build the project crate list, which excludes sysroot context crates
    // (rust-analyzer discovers those from sysroot_src).
    let project_crate_names = collect_project_crate_names(&registry);

    // Resolve target spec path for kernel crates.
    let kernel_target = config
        .root
        .join(&config.target.spec)
        .to_str()
        .expect("target spec path is valid UTF-8")
        .to_string();

    let mut entries = Vec::new();
    for krate in &all_crates {
        let deps = krate
            .deps
            .iter()
            .filter_map(|dep| {
                name_to_idx.get(&dep.crate_name).map(|&idx| DepEntry {
                    crate_idx: idx,
                    name: dep.extern_name.clone(),
                })
            })
            .collect();

        let mut cfg = Vec::new();

        // Add feature cfgs.
        for feat in &krate.features {
            cfg.push(format!("feature=\"{feat}\""));
        }

        // Add config cfgs for kernel crates.
        if krate.context != CrateContext::Host {
            cfg.extend(config_cfgs.iter().cloned());
        }

        // Per-crate target: host crates use host default (None),
        // kernel crates use the custom target spec.
        let target = if krate.context == CrateContext::Host {
            None
        } else {
            Some(kernel_target.clone())
        };

        let is_workspace = project_crate_names.contains(&krate.name);

        let proc_macro_dylib = if krate.crate_type == "proc-macro" {
            // Point to the compiled dylib in build/host/.
            let ext = if cfg!(target_os = "macos") {
                "dylib"
            } else {
                "so"
            };
            let dylib = config.root.join(format!(
                "build/host/lib{}.{ext}",
                compile::crate_name_sanitized(&krate.name)
            ));
            if dylib.exists() { Some(dylib) } else { None }
        } else {
            None
        };

        entries.push(CrateEntry {
            display_name: krate.name.clone(),
            root_module: krate.root_file.clone(),
            edition: krate.edition.clone(),
            deps,
            cfg,
            target,
            is_proc_macro: krate.crate_type == "proc-macro",
            proc_macro_dylib_path: proc_macro_dylib,
            is_workspace_member: is_workspace,
        });
    }

    let project = RustProject {
        sysroot_src,
        crates: entries,
    };

    let output_path = config.root.join("rust-project.json");
    let json = serde_json::to_string_pretty(&project)
        .context("serializing rust-project.json")?;
    std::fs::write(&output_path, json)
        .with_context(|| format!("writing {}", output_path.display()))?;

    println!("Generated {} ({} crates)", output_path.display(), all_crates.len());
    Ok(output_path)
}

/// Build --cfg flags from resolved config bool options.
fn build_config_cfgs(config: &ResolvedConfig) -> Vec<String> {
    let mut cfgs = Vec::new();
    for (name, value) in &config.options {
        if let ResolvedValue::Bool(true) = value {
            cfgs.push(format!("hadron_{name}"));
        }
    }
    cfgs
}

/// Collect names of project crates (not vendored, not sysroot).
fn collect_project_crate_names(registry: &CrateRegistry) -> Vec<String> {
    let project_prefixes = ["kernel/", "crates/", "userspace/"];
    registry
        .crates
        .iter()
        .filter(|(_, def)| {
            let path = &def.path;
            project_prefixes.iter().any(|p| path.starts_with(p))
        })
        .map(|(name, _)| name.clone())
        .collect()
}
