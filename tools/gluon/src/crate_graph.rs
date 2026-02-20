//! Crate graph resolution from the build model.
//!
//! Converts model crate definitions into resolved crates, builds a dependency
//! graph, and produces a topologically sorted compilation order using Kahn's
//! algorithm.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// A resolved crate ready for compilation.
#[derive(Debug)]
pub struct ResolvedCrate {
    pub name: String,
    pub edition: String,
    pub crate_type: String,
    /// Target for this crate. `"host"` = host triple.
    pub target: String,
    pub deps: Vec<ResolvedDep>,
    pub features: Vec<String>,
    pub root_file: PathBuf,
    /// Per-crate linker script path (relative to project root).
    pub linker_script: Option<String>,
    /// Whether this crate is a project crate (for clippy linting).
    pub is_project_crate: bool,
    /// Extra `--cfg` flags for this crate.
    pub cfg_flags: Vec<String>,
}

/// A resolved dependency.
#[derive(Debug)]
pub struct ResolvedDep {
    pub extern_name: String,
    pub crate_name: String,
}

/// Resolve a crate path, replacing `{sysroot}` with the actual sysroot source dir.
fn resolve_path(raw: &str, root: &Path, sysroot_src: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("{sysroot}/") {
        sysroot_src.join(rest)
    } else if let Some(name) = raw.strip_prefix("vendor/") {
        crate::vendor::find_vendor_dir(name, &root.join("vendor"))
    } else {
        root.join(raw)
    }
}

/// Resolve crates from a model group, then topologically sort them.
///
/// Converts model [`CrateDef`](crate::model::CrateDef)s into [`ResolvedCrate`]s
/// and toposorts using Kahn's algorithm.
pub fn resolve_group_from_model(
    model: &crate::model::BuildModel,
    group_name: &str,
    root: &Path,
    sysroot_src: &Path,
) -> Result<Vec<ResolvedCrate>> {
    let group = model.groups.get(group_name)
        .with_context(|| format!("group '{group_name}' not found in model"))?;

    // Collect all crates in this group.
    let matching: Vec<(&str, &crate::model::CrateDef)> = group
        .crates
        .iter()
        .filter_map(|name| model.crates.get(name).map(|c| (name.as_str(), c)))
        .collect();

    // Build adjacency and in-degree for Kahn's algorithm.
    let name_set: HashSet<&str> = matching.iter().map(|(n, _)| *n).collect();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for (name, _) in &matching {
        in_degree.insert(name, 0);
    }

    for (name, def) in &matching {
        for dep in def.deps.values() {
            let dep_name = dep.crate_name.as_str();
            if name_set.contains(dep_name) {
                *in_degree.entry(name).or_insert(0) += 1;
                dependents.entry(dep_name).or_default().push(name);
            }
        }
    }

    // Kahn's algorithm.
    let mut queue: VecDeque<&str> = VecDeque::new();
    for (name, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(name);
        }
    }

    let mut sorted_names: Vec<String> = Vec::new();
    while let Some(name) = queue.pop_front() {
        sorted_names.push(name.to_string());
        if let Some(deps) = dependents.get(name) {
            for dep in deps {
                if let Some(degree) = in_degree.get_mut(dep) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(dep);
                    }
                }
            }
        }
    }

    if sorted_names.len() != matching.len() {
        bail!(
            "dependency cycle detected among crates in group '{group_name}'"
        );
    }

    // Build resolved crates in sorted order.
    let mut resolved = Vec::new();
    for name in &sorted_names {
        let def = model.crates.get(name).unwrap();
        let crate_path = resolve_path(&def.path, root, sysroot_src);
        let rf = def.root_file(&crate_path);
        let crate_type = def.crate_type.as_str().to_string();

        let deps: Vec<ResolvedDep> = def
            .deps
            .iter()
            .map(|(extern_name, dep)| ResolvedDep {
                extern_name: extern_name.clone(),
                crate_name: dep.crate_name.clone(),
            })
            .collect();

        resolved.push(ResolvedCrate {
            name: name.clone(),
            edition: def.edition.clone(),
            crate_type,
            target: def.target.clone(),
            deps,
            features: def.features.clone(),
            root_file: rf,
            linker_script: def.linker_script.clone(),
            is_project_crate: def.is_project_crate,
            cfg_flags: def.cfg_flags.clone(),
        });
    }

    Ok(resolved)
}
