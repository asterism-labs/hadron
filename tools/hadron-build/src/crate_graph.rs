//! Crate graph resolution from `crates.toml`.
//!
//! Parses the crate registry, builds a dependency graph, and produces a
//! topologically sorted compilation order using Kahn's algorithm.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// A single crate entry from `crates.toml`.
#[derive(Debug, Deserialize)]
pub struct CrateDef {
    pub path: String,
    #[serde(default = "default_edition")]
    pub edition: String,
    #[serde(default, rename = "type")]
    pub crate_type: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub deps: BTreeMap<String, DepSpec>,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub root: Option<String>,
}

fn default_edition() -> String {
    "2024".into()
}

/// A dependency specification: either a simple string or a table.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DepSpec {
    /// Simple: `extern_name = "crate-name"`.
    Simple(String),
    /// Table: `extern_name = { crate = "name", features = [...], proc-macro = true }`.
    Table {
        #[serde(rename = "crate")]
        crate_name: String,
        #[allow(dead_code)] // used by feature-gated compilation
        #[serde(default)]
        features: Vec<String>,
        #[serde(default, rename = "proc-macro")]
        proc_macro: bool,
    },
}

impl DepSpec {
    /// Get the crate name this dep points to.
    pub fn crate_name(&self) -> &str {
        match self {
            DepSpec::Simple(name) => name,
            DepSpec::Table { crate_name, .. } => crate_name,
        }
    }

    /// Whether this dep is a proc-macro.
    pub fn is_proc_macro(&self) -> bool {
        matches!(self, DepSpec::Table { proc_macro: true, .. })
    }
}

/// The full crate registry parsed from `crates.toml`.
#[derive(Debug, Deserialize)]
pub struct CrateRegistry {
    #[serde(rename = "crate")]
    pub crates: BTreeMap<String, CrateDef>,
}

/// A resolved crate ready for compilation.
#[derive(Debug)]
pub struct ResolvedCrate {
    pub name: String,
    pub path: PathBuf,
    pub edition: String,
    pub crate_type: String,
    pub context: CrateContext,
    pub deps: Vec<ResolvedDep>,
    pub features: Vec<String>,
    pub root_file: PathBuf,
}

/// Where a crate should be compiled: sysroot, host, userspace, or kernel target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrateContext {
    Sysroot,
    Host,
    Kernel,
    Userspace,
}

/// A resolved dependency.
#[derive(Debug)]
pub struct ResolvedDep {
    pub extern_name: String,
    pub crate_name: String,
    pub is_proc_macro: bool,
}

/// Parse `crates.toml` and resolve all crate definitions.
pub fn load_crate_registry(root: &Path) -> Result<CrateRegistry> {
    let path = root.join("crates.toml");
    let contents =
        std::fs::read_to_string(&path).context("failed to read crates.toml")?;
    toml::from_str(&contents).context("failed to parse crates.toml")
}

/// Resolve a crate path, replacing `{sysroot}` with the actual sysroot source dir.
fn resolve_path(raw: &str, root: &Path, sysroot_src: &Path) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("{sysroot}/") {
        sysroot_src.join(rest)
    } else {
        root.join(raw)
    }
}

/// Determine the root source file for a crate.
fn root_file(crate_def: &CrateDef, resolved_path: &Path) -> PathBuf {
    if let Some(ref root) = crate_def.root {
        resolved_path.join(root)
    } else {
        let crate_type = crate_def
            .crate_type
            .as_deref()
            .unwrap_or("lib");
        if crate_type == "bin" {
            resolved_path.join("src/main.rs")
        } else {
            resolved_path.join("src/lib.rs")
        }
    }
}

/// Filter and resolve crates by context, then topologically sort them.
pub fn resolve_and_sort(
    registry: &CrateRegistry,
    root: &Path,
    sysroot_src: &Path,
    context_filter: &CrateContext,
) -> Result<Vec<ResolvedCrate>> {
    // Collect crates matching this context.
    let matching: Vec<(&String, &CrateDef)> = registry
        .crates
        .iter()
        .filter(|(_, def)| {
            let ctx = match def.context.as_deref() {
                Some("sysroot") => CrateContext::Sysroot,
                Some("host") => CrateContext::Host,
                Some("userspace") => CrateContext::Userspace,
                _ => CrateContext::Kernel,
            };
            ctx == *context_filter
        })
        .collect();

    // Build adjacency and in-degree for Kahn's algorithm.
    let name_set: HashSet<&str> = matching.iter().map(|(n, _)| n.as_str()).collect();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for (name, _) in &matching {
        in_degree.insert(name.as_str(), 0);
    }

    for (name, def) in &matching {
        for dep_spec in def.deps.values() {
            let dep_name = dep_spec.crate_name();
            // Only count in-context dependencies (proc-macro deps are cross-context).
            if name_set.contains(dep_name) {
                *in_degree.entry(name.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep_name)
                    .or_default()
                    .push(name.as_str());
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
            "dependency cycle detected among {:?} context crates",
            context_filter
        );
    }

    // Build resolved crates in sorted order.
    let mut resolved = Vec::new();
    for name in &sorted_names {
        let def = &registry.crates[name];
        let crate_path = resolve_path(&def.path, root, sysroot_src);
        let rf = root_file(def, &crate_path);
        let crate_type = def
            .crate_type
            .as_deref()
            .unwrap_or("lib")
            .to_string();
        let context = match def.context.as_deref() {
            Some("sysroot") => CrateContext::Sysroot,
            Some("host") => CrateContext::Host,
            Some("userspace") => CrateContext::Userspace,
            _ => CrateContext::Kernel,
        };

        let deps: Vec<ResolvedDep> = def
            .deps
            .iter()
            .map(|(extern_name, spec)| ResolvedDep {
                extern_name: extern_name.clone(),
                crate_name: spec.crate_name().to_string(),
                is_proc_macro: spec.is_proc_macro(),
            })
            .collect();

        resolved.push(ResolvedCrate {
            name: name.clone(),
            path: crate_path,
            edition: def.edition.clone(),
            crate_type,
            context,
            deps,
            features: def.features.clone(),
            root_file: rf,
        });
    }

    Ok(resolved)
}
