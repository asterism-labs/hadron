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
use crate::crate_graph::{self, ResolvedCrate};
use crate::model::BuildModel;
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

/// Generate `rust-project.json` from a [`BuildModel`].
pub fn generate_rust_project(
    config: &ResolvedConfig,
    model: &BuildModel,
) -> Result<PathBuf> {
    let sysroot_src = sysroot::sysroot_src_dir()?;

    // Resolve host crates.
    let host_crates = resolve_groups_by_target(model, "host", config, &sysroot_src)?;

    // Collect all non-host groups (kernel, userspace, vendored, etc.).
    let non_host_group_names: Vec<String> = model.groups.iter()
        .filter(|(_, g)| g.target != "host")
        .map(|(name, _)| name.clone())
        .collect();

    let mut non_host_crates = Vec::new();
    for gname in &non_host_group_names {
        let group_crates = crate_graph::resolve_group_from_model(
            model, gname, &config.root, &sysroot_src,
        )?;
        for krate in group_crates {
            if !non_host_crates.iter().any(|k: &ResolvedCrate| k.name == krate.name) {
                non_host_crates.push(krate);
            }
        }
    }

    // Build combined crate list. Order: host crates first (proc-macros), then non-host.
    let mut all_crates: Vec<&ResolvedCrate> = Vec::new();
    let mut name_to_idx: BTreeMap<String, usize> = BTreeMap::new();

    for krate in &host_crates {
        let idx = all_crates.len();
        name_to_idx.insert(krate.name.clone(), idx);
        all_crates.push(krate);
    }

    for krate in &non_host_crates {
        if !name_to_idx.contains_key(&krate.name) {
            let idx = all_crates.len();
            name_to_idx.insert(krate.name.clone(), idx);
            all_crates.push(krate);
        }
    }

    // Build cfg flags from resolved config options.
    let config_cfgs = build_config_cfgs(config);

    // Collect project crate names from model.
    let project_crate_names: Vec<String> = model.crates.iter()
        .filter(|(_, c)| c.is_project_crate)
        .map(|(name, _)| name.clone())
        .collect();

    // Collect config-enabled groups.
    let config_groups: std::collections::HashSet<String> = model.groups.iter()
        .filter(|(_, g)| g.config)
        .map(|(name, _)| name.clone())
        .collect();

    // Resolve target spec paths for non-host crates.
    let mut target_spec_paths: BTreeMap<String, String> = BTreeMap::new();
    for (tname, tdef) in &model.targets {
        let spec_path = config
            .root
            .join(&tdef.spec)
            .to_str()
            .expect("target spec path is valid UTF-8")
            .to_string();
        target_spec_paths.insert(tname.clone(), spec_path);
    }

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
        for feat in &krate.features {
            cfg.push(format!("feature=\"{feat}\""));
        }

        // Add config cfgs for crates in config-enabled groups.
        let crate_group = model.crates.get(&krate.name)
            .and_then(|c| c.group.as_ref());
        let in_config_group = crate_group
            .map(|g| config_groups.contains(g))
            .unwrap_or(false);
        if krate.target != "host" && in_config_group {
            cfg.extend(config_cfgs.iter().cloned());
        }

        let target = if krate.target == "host" {
            None
        } else {
            target_spec_paths.get(&krate.target).cloned()
        };

        let is_workspace = project_crate_names.contains(&krate.name);

        let proc_macro_dylib = if krate.crate_type == "proc-macro" {
            let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
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

/// Resolve all crates from groups matching a given target.
fn resolve_groups_by_target(
    model: &BuildModel,
    target: &str,
    config: &ResolvedConfig,
    sysroot_src: &std::path::Path,
) -> Result<Vec<ResolvedCrate>> {
    let group_names: Vec<String> = model.groups.iter()
        .filter(|(_, g)| g.target == target)
        .map(|(name, _)| name.clone())
        .collect();

    let mut crates = Vec::new();
    for gname in &group_names {
        let group_crates = crate_graph::resolve_group_from_model(
            model, gname, &config.root, sysroot_src,
        )?;
        for krate in group_crates {
            if !crates.iter().any(|k: &ResolvedCrate| k.name == krate.name) {
                crates.push(krate);
            }
        }
    }
    Ok(crates)
}

/// Build --cfg flags from resolved config options respecting bindings.
fn build_config_cfgs(config: &ResolvedConfig) -> Vec<String> {
    use crate::model::Binding;

    let mut cfgs = Vec::new();
    for (name, value) in &config.options {
        let opt_bindings = config.bindings.get(name);

        let has_cfg = opt_bindings.map_or(false, |bs| bs.contains(&Binding::Cfg));
        let has_cfg_cumulative = opt_bindings.map_or(false, |bs| bs.contains(&Binding::CfgCumulative));
        let is_legacy = opt_bindings.is_none();

        if has_cfg {
            match value {
                ResolvedValue::Bool(true) => {
                    cfgs.push(format!("hadron_{name}"));
                }
                ResolvedValue::Choice(v) | ResolvedValue::Str(v) => {
                    cfgs.push(format!("hadron_{name}=\"{v}\""));
                }
                _ => {}
            }
        } else if has_cfg_cumulative {
            if let ResolvedValue::Choice(selected) = value {
                cfgs.push(format!("hadron_{name}=\"{selected}\""));
                if let Some(variants) = config.choices.get(name) {
                    if let Some(selected_idx) = variants.iter().position(|v| v == selected) {
                        for variant in &variants[..=selected_idx] {
                            cfgs.push(format!("hadron_{name}_{variant}"));
                        }
                    }
                }
            }
        } else if is_legacy {
            if let ResolvedValue::Bool(true) = value {
                cfgs.push(format!("hadron_{name}"));
            }
        }
    }
    cfgs
}
