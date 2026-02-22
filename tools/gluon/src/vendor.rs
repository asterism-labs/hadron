//! Dependency vendoring: Cargo.toml parsing, transitive resolution, fetching,
//! lock file management, and auto-registration into the build model.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result, bail};

// ---------------------------------------------------------------------------
// Cargo.toml parsing
// ---------------------------------------------------------------------------

/// Parsed package metadata from a Cargo.toml `[package]` section.
#[derive(Debug)]
pub struct CargoPackage {
    pub name: String,
    pub version: String,
    pub edition: String,
}

/// A dependency entry parsed from Cargo.toml `[dependencies]`.
#[derive(Debug)]
pub struct CargoDep {
    pub name: String,
    /// The Cargo.toml key (may differ from package name via `package = "..."`)
    pub key: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub features: Vec<String>,
    pub default_features: bool,
    pub optional: bool,
}

/// Crate type extracted from Cargo.toml `[lib]` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoCrateType {
    Lib,
    ProcMacro,
}

/// Parsed Cargo.toml for dependency resolution.
#[derive(Debug)]
pub struct ParsedCargoToml {
    pub package: CargoPackage,
    pub dependencies: Vec<CargoDep>,
    pub features: BTreeMap<String, Vec<String>>,
    pub default_features: Vec<String>,
    pub crate_type: CargoCrateType,
    /// The `[lib]` name override, if any.
    #[allow(dead_code)] // used by future crate resolution improvements
    pub lib_name: Option<String>,
}

/// Parse a Cargo.toml file at the given path.
pub fn parse_cargo_toml(path: &Path) -> Result<ParsedCargoToml> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    parse_cargo_toml_str(&content, path)
}

/// Parse Cargo.toml from a string (for testing).
fn parse_cargo_toml_str(content: &str, path: &Path) -> Result<ParsedCargoToml> {
    let doc: toml::Table = content.parse()
        .with_context(|| format!("parsing {}", path.display()))?;

    let package = parse_package_section(&doc, path)?;
    let dependencies = parse_dependencies_section(&doc);
    let (features, default_features) = parse_features_section(&doc);
    let (crate_type, lib_name) = parse_lib_section(&doc);

    Ok(ParsedCargoToml {
        package,
        dependencies,
        features,
        default_features,
        crate_type,
        lib_name,
    })
}

fn parse_package_section(doc: &toml::Table, path: &Path) -> Result<CargoPackage> {
    let pkg = doc.get("package")
        .and_then(|v| v.as_table())
        .ok_or_else(|| anyhow::anyhow!("[package] section missing in {}", path.display()))?;

    let name = pkg.get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("package.name missing in {}", path.display()))?
        .to_string();

    let version = pkg.get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    let edition = pkg.get("edition")
        .and_then(|v| v.as_str())
        .unwrap_or("2021")
        .to_string();

    Ok(CargoPackage { name, version, edition })
}

fn parse_dependencies_section(doc: &toml::Table) -> Vec<CargoDep> {
    let mut deps = Vec::new();

    let Some(dep_section) = doc.get("dependencies") else {
        return deps;
    };

    let Some(dep_table) = dep_section.as_table() else {
        return deps;
    };

    for (key, value) in dep_table {
        let dep = parse_single_dep(key, value);
        deps.push(dep);
    }

    deps
}

fn parse_single_dep(key: &str, value: &toml::Value) -> CargoDep {
    match value {
        // Simple form: `name = "version"`
        toml::Value::String(version) => CargoDep {
            name: key.to_string(),
            key: key.to_string(),
            version: Some(version.clone()),
            path: None,
            features: Vec::new(),
            default_features: true,
            optional: false,
        },
        // Table form: `[dependencies.name]` with fields
        toml::Value::Table(table) => {
            let package_name = table.get("package")
                .and_then(|v| v.as_str())
                .unwrap_or(key)
                .to_string();

            let version = table.get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let path = table.get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let features = table.get("features")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let default_features = table.get("default-features")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let optional = table.get("optional")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            CargoDep {
                name: package_name,
                key: key.to_string(),
                version,
                path,
                features,
                default_features,
                optional,
            }
        }
        _ => CargoDep {
            name: key.to_string(),
            key: key.to_string(),
            version: None,
            path: None,
            features: Vec::new(),
            default_features: true,
            optional: false,
        },
    }
}

fn parse_features_section(doc: &toml::Table) -> (BTreeMap<String, Vec<String>>, Vec<String>) {
    let mut features = BTreeMap::new();
    let mut default = Vec::new();

    let Some(feat_section) = doc.get("features") else {
        return (features, default);
    };

    let Some(feat_table) = feat_section.as_table() else {
        return (features, default);
    };

    for (key, value) in feat_table {
        let specs: Vec<String> = value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        if key == "default" {
            default = specs.clone();
        }
        features.insert(key.clone(), specs);
    }

    (features, default)
}

fn parse_lib_section(doc: &toml::Table) -> (CargoCrateType, Option<String>) {
    let Some(lib) = doc.get("lib").and_then(|v| v.as_table()) else {
        return (CargoCrateType::Lib, None);
    };

    let is_proc_macro = lib.get("proc-macro")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let lib_name = lib.get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let crate_type = if is_proc_macro {
        CargoCrateType::ProcMacro
    } else {
        CargoCrateType::Lib
    };

    (crate_type, lib_name)
}

// ---------------------------------------------------------------------------
// Transitive dependency resolution
// ---------------------------------------------------------------------------

/// A fully resolved dependency ready for vendoring.
#[derive(Debug, Clone)]
pub struct ResolvedDep {
    pub name: String,
    pub version: String,
    pub source: ResolvedSource,
    pub features: Vec<String>,
    pub is_proc_macro: bool,
    /// Dependency edges: what this crate depends on.
    pub deps: Vec<ResolvedDepEdge>,
    /// Parent crate names that caused this dep to be included.
    pub required_by: Vec<String>,
}

/// A resolved dependency edge from one crate to another.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields are part of the public dep-tree API for diagnostics and tooling.
pub struct ResolvedDepEdge {
    /// Dependency crate name.
    pub name: String,
    /// Version requirement from Cargo.toml (e.g. "1.0.0", "^0.4").
    pub version_req: String,
    /// Rust extern name (hyphens replaced with underscores).
    pub extern_name: String,
}

/// The resolved source of a dependency.
#[derive(Debug, Clone)]
pub enum ResolvedSource {
    CratesIo,
    Git { url: String, reference: String },
    Path { path: String },
}

/// Resolve all transitive dependencies starting from root declarations.
///
/// Returns a topologically sorted list of all dependencies that need to be
/// vendored, with unified features. Performs version-aware deduplication and
/// conflict detection.
///
/// `locked_versions` provides pinned versions from the lock file. When a
/// non-exact version requirement is resolved, the lock file is consulted
/// first before querying crates.io, ensuring deterministic builds.
pub fn resolve_transitive(
    roots: &BTreeMap<String, crate::model::ExternalDepDef>,
    vendor_dir: &Path,
    version_cache: &mut VersionCache,
    locked_versions: &HashMap<String, String>,
) -> Result<Vec<ResolvedDep>> {
    let mut resolved: BTreeMap<String, ResolvedDep> = BTreeMap::new();
    // Parallel HashSet per dep for O(1) feature containment checks.
    let mut feature_sets: HashMap<String, HashSet<String>> = HashMap::new();
    let mut queue: std::collections::VecDeque<QueueEntry> = std::collections::VecDeque::new();

    // Seed the queue with root dependencies.
    for (name, ext_dep) in roots {
        let (version, source) = match &ext_dep.source {
            crate::model::DepSource::CratesIo { version } => {
                (version.clone(), ResolvedSource::CratesIo)
            }
            crate::model::DepSource::Git { url, reference } => {
                let ref_str = match reference {
                    crate::model::GitRef::Rev(r) => r.clone(),
                    crate::model::GitRef::Tag(t) => t.clone(),
                    crate::model::GitRef::Branch(b) => b.clone(),
                    crate::model::GitRef::Default => "HEAD".into(),
                };
                (String::new(), ResolvedSource::Git { url: url.clone(), reference: ref_str })
            }
            crate::model::DepSource::Path { path } => {
                (String::new(), ResolvedSource::Path { path: path.clone() })
            }
        };

        let mut initial_features = Vec::new();
        if ext_dep.default_features {
            initial_features.push("__default__".to_string());
        }
        initial_features.extend(ext_dep.features.clone());

        queue.push_back(QueueEntry {
            name: name.clone(),
            version,
            source,
            requested_features: initial_features,
            required_by: "(root)".to_string(),
        });
    }

    // BFS resolution.
    while let Some(entry) = queue.pop_front() {
        if let Some(existing) = resolved.get_mut(&entry.name) {
            // Version-aware dedup: check for conflicts when versions differ.
            if !existing.version.is_empty()
                && !entry.version.is_empty()
                && existing.version != entry.version
            {
                if !versions_compatible(&existing.version, &entry.version) {
                    bail!(
                        "version conflict for '{}': '{}' requires {}, but '{}' requires {}",
                        entry.name,
                        entry.required_by, entry.version,
                        existing.required_by.join(", "), existing.version,
                    );
                }
                // Keep the higher compatible version.
                if let (Ok(new_ver), Ok(old_ver)) = (
                    semver::Version::parse(&entry.version),
                    semver::Version::parse(&existing.version),
                ) {
                    if new_ver > old_ver {
                        existing.version = entry.version.clone();
                    }
                }
            }

            // Track additional required_by.
            if !existing.required_by.contains(&entry.required_by) {
                existing.required_by.push(entry.required_by.clone());
            }

            // Unify features using HashSet for O(1) checks.
            let feat_set = feature_sets.get_mut(&entry.name).unwrap();
            let mut changed = false;
            for feat in &entry.requested_features {
                if feat_set.insert(feat.clone()) {
                    existing.features.push(feat.clone());
                    changed = true;
                }
            }
            if !changed {
                // No new features to propagate.
                continue;
            }
            // Features changed — need to re-process this dep's transitive deps.
        } else {
            let feat_set: HashSet<String> = entry.requested_features.iter().cloned().collect();
            feature_sets.insert(entry.name.clone(), feat_set);
            resolved.insert(entry.name.clone(), ResolvedDep {
                name: entry.name.clone(),
                version: entry.version.clone(),
                source: entry.source.clone(),
                features: entry.requested_features.clone(),
                is_proc_macro: false,
                deps: Vec::new(),
                required_by: vec![entry.required_by.clone()],
            });
        }

        // Find the vendored Cargo.toml to discover transitive deps.
        let vendor_path = find_vendor_dir(&entry.name, Some(&entry.version), vendor_dir);
        let cargo_toml_path = vendor_path.join("Cargo.toml");
        if !cargo_toml_path.exists() {
            // Not yet vendored — will be fetched later. Skip transitive resolution
            // for now; the vendor command will iterate until stable.
            continue;
        }

        let parsed = parse_cargo_toml(&cargo_toml_path)
            .with_context(|| format!("parsing transitive dep {}", entry.name))?;

        // Update version and proc-macro status from parsed Cargo.toml.
        if let Some(dep) = resolved.get_mut(&entry.name) {
            if dep.version.is_empty() {
                dep.version = parsed.package.version.clone();
            }
            dep.is_proc_macro = parsed.crate_type == CargoCrateType::ProcMacro;
        }

        // Compute activated features for this dependency.
        let dep_info = resolved.get(&entry.name).unwrap();
        let activated = compute_activated_features(
            &dep_info.features,
            &parsed.features,
            &parsed.default_features,
        );

        // Collect dependency edges for this crate.
        let mut dep_edges: Vec<ResolvedDepEdge> = Vec::new();

        // Enqueue transitive dependencies.
        for cargo_dep in &parsed.dependencies {
            if cargo_dep.optional {
                // Only include if activated by a feature.
                let dep_key = &cargo_dep.key;
                let dep_feat_key = format!("dep:{}", dep_key);
                if !activated.contains(dep_key) && !activated.contains(&dep_feat_key) {
                    continue;
                }
            }

            // Determine features to pass to this transitive dep.
            let mut trans_features = Vec::new();
            let mut trans_feat_set = HashSet::new();
            if cargo_dep.default_features {
                trans_feat_set.insert("__default__".to_string());
                trans_features.push("__default__".to_string());
            }
            for feat in &cargo_dep.features {
                if trans_feat_set.insert(feat.clone()) {
                    trans_features.push(feat.clone());
                }
            }

            // Propagate features from parent feature specs (e.g. "dep/feature").
            for feat_spec in &activated {
                if let Some(rest) = feat_spec.strip_prefix(&format!("{}/", cargo_dep.key)) {
                    let rest_owned = rest.to_string();
                    if trans_feat_set.insert(rest_owned.clone()) {
                        trans_features.push(rest_owned);
                    }
                }
            }

            let source = if cargo_dep.path.is_some() {
                // Path deps within a vendored crate — resolve relative to vendor.
                ResolvedSource::CratesIo
            } else {
                ResolvedSource::CratesIo
            };

            let raw_version = cargo_dep.version.clone().unwrap_or_default();
            let resolved_version = if !raw_version.is_empty()
                && matches!(source, ResolvedSource::CratesIo)
            {
                resolve_version_with_lock(
                    &cargo_dep.name,
                    &raw_version,
                    version_cache,
                    locked_versions,
                )?
            } else {
                raw_version.clone()
            };

            // Record the dependency edge.
            dep_edges.push(ResolvedDepEdge {
                name: cargo_dep.name.clone(),
                version_req: raw_version,
                extern_name: cargo_dep.key.replace('-', "_"),
            });

            queue.push_back(QueueEntry {
                name: cargo_dep.name.clone(),
                version: resolved_version,
                source,
                requested_features: trans_features,
                required_by: entry.name.clone(),
            });
        }

        // Store dependency edges on the resolved dep.
        if let Some(dep) = resolved.get_mut(&entry.name) {
            dep.deps = dep_edges;
        }
    }

    // Convert "__default__" markers into actual default features.
    for dep in resolved.values_mut() {
        let vendor_path = find_vendor_dir(&dep.name, Some(&dep.version), vendor_dir);
        let cargo_toml_path = vendor_path.join("Cargo.toml");
        if cargo_toml_path.exists() {
            if let Ok(parsed) = parse_cargo_toml(&cargo_toml_path) {
                let mut expanded = Vec::new();
                for feat in &dep.features {
                    if feat == "__default__" {
                        expanded.extend(parsed.default_features.clone());
                    } else {
                        expanded.push(feat.clone());
                    }
                }
                // Dedup while preserving order.
                let mut seen = HashSet::new();
                expanded.retain(|f| seen.insert(f.clone()));
                dep.features = expanded;
            }
        }
    }

    // Return sorted by name for determinism.
    let mut result: Vec<ResolvedDep> = resolved.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

struct QueueEntry {
    name: String,
    version: String,
    source: ResolvedSource,
    requested_features: Vec<String>,
    /// Which crate caused this dep to be queued.
    required_by: String,
}

/// Check if two semver versions are compatible (same major for >= 1.0, same
/// major.minor for 0.x).
fn versions_compatible(a: &str, b: &str) -> bool {
    let Ok(va) = semver::Version::parse(a) else { return false };
    let Ok(vb) = semver::Version::parse(b) else { return false };
    if va.major == 0 && vb.major == 0 {
        va.minor == vb.minor
    } else {
        va.major == vb.major
    }
}

/// Resolve a version, checking the lock file first for determinism.
fn resolve_version_with_lock(
    name: &str,
    version_str: &str,
    cache: &mut VersionCache,
    locked_versions: &HashMap<String, String>,
) -> Result<String> {
    // If already an exact version, return immediately.
    if semver::Version::parse(version_str).is_ok() {
        return Ok(version_str.to_string());
    }

    // Check the lock file first for determinism.
    if let Some(locked_ver) = locked_versions.get(name) {
        if let Ok(req) = version_str.parse::<semver::VersionReq>() {
            if let Ok(ver) = semver::Version::parse(locked_ver) {
                if req.matches(&ver) {
                    return Ok(locked_ver.clone());
                }
            }
        }
    }

    // Fall back to crates.io resolution.
    resolve_version(name, version_str, cache)
}

/// Compute the set of activated feature specs given requested features
/// and the crate's feature table.
fn compute_activated_features(
    requested: &[String],
    feature_table: &BTreeMap<String, Vec<String>>,
    default_features: &[String],
) -> Vec<String> {
    let mut activated = Vec::new();
    let mut work: Vec<String> = Vec::new();

    for feat in requested {
        if feat == "__default__" {
            work.extend(default_features.iter().cloned());
        } else {
            work.push(feat.clone());
        }
    }

    let mut seen = HashSet::new();
    while let Some(feat) = work.pop() {
        if !seen.insert(feat.clone()) {
            continue;
        }
        activated.push(feat.clone());

        // Expand feature to its sub-specs.
        if let Some(specs) = feature_table.get(&feat) {
            for spec in specs {
                work.push(spec.clone());
            }
        }
    }

    activated
}

/// Find the vendor directory for a dependency.
///
/// When `version` is provided, checks for the exact `vendor/{name}-{version}/`
/// directory first. Falls back to a prefix scan that skips directories without
/// `Cargo.toml` (corrupt/incomplete). Final fallback: `vendor/{name}/`.
pub fn find_vendor_dir(name: &str, version: Option<&str>, vendor_dir: &Path) -> std::path::PathBuf {
    // Exact version match first.
    if let Some(ver) = version {
        let exact = vendor_dir.join(format!("{name}-{ver}"));
        if exact.is_dir() && exact.join("Cargo.toml").exists() {
            return exact;
        }
    }

    // Fallback: prefix scan, but ONLY dirs with Cargo.toml.
    if let Ok(entries) = std::fs::read_dir(vendor_dir) {
        let prefix = format!("{name}-");
        for entry in entries.flatten() {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy();
            if fname_str.starts_with(&prefix)
                && entry.path().is_dir()
                && entry.path().join("Cargo.toml").exists()
            {
                return entry.path();
            }
        }
    }

    // Fall back to unversioned.
    vendor_dir.join(name)
}

// ---------------------------------------------------------------------------
// Workspace reference resolution
// ---------------------------------------------------------------------------

/// Metadata extracted from a workspace root `Cargo.toml`.
struct WorkspaceMetadata {
    /// `[workspace.package]` fields (version, edition, authors, etc.).
    package: toml::Table,
    /// `[workspace.dependencies]` table mapping dep name → dep spec.
    dependencies: toml::Table,
}

/// Parse `[workspace.package]` and `[workspace.dependencies]` from a workspace
/// root `Cargo.toml` table.
fn parse_workspace_metadata(root: &toml::Table) -> WorkspaceMetadata {
    let ws = root.get("workspace").and_then(|v| v.as_table());

    let package = ws
        .and_then(|w| w.get("package"))
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();

    let dependencies = ws
        .and_then(|w| w.get("dependencies"))
        .and_then(|v| v.as_table())
        .cloned()
        .unwrap_or_default();

    WorkspaceMetadata { package, dependencies }
}

/// Check whether a TOML value is `{ workspace = true }`.
fn is_workspace_inherited(value: &toml::Value) -> bool {
    value
        .as_table()
        .and_then(|t| t.get("workspace"))
        .and_then(|v| v.as_bool())
        == Some(true)
}

/// Rewrite all `workspace = true` references in a member `Cargo.toml` with
/// concrete values from the workspace root, returning the normalized TOML
/// string. Behaves like `cargo publish` — the vendored crate becomes
/// self-contained.
fn resolve_workspace_references(
    member: &mut toml::Table,
    meta: &WorkspaceMetadata,
) {
    resolve_package_workspace_fields(member, &meta.package);

    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        resolve_deps_workspace_refs(member, section, &meta.dependencies);
    }
}

/// Replace `version.workspace = true`, `edition.workspace = true`, etc. in
/// `[package]` with values from `[workspace.package]`.
fn resolve_package_workspace_fields(member: &mut toml::Table, ws_pkg: &toml::Table) {
    let Some(pkg) = member.get_mut("package").and_then(|v| v.as_table_mut()) else {
        return;
    };

    // Fields that support `key.workspace = true` inheritance.
    let inheritable: &[&str] = &[
        "version", "authors", "description", "documentation", "readme",
        "homepage", "repository", "license", "license-file", "keywords",
        "categories", "edition", "rust-version", "exclude", "include",
        "publish",
    ];

    for &field in inheritable {
        let Some(val) = pkg.get(field) else { continue };
        if is_workspace_inherited(val) {
            if let Some(ws_val) = ws_pkg.get(field) {
                pkg.insert(field.to_string(), ws_val.clone());
            } else {
                // No workspace value — remove the broken reference.
                pkg.remove(field);
            }
        }
    }
}

/// Replace `dep = { workspace = true, ... }` entries in a dependency section
/// with the concrete spec from `[workspace.dependencies]`.
fn resolve_deps_workspace_refs(
    member: &mut toml::Table,
    section: &str,
    ws_deps: &toml::Table,
) {
    let Some(deps) = member.get_mut(section).and_then(|v| v.as_table_mut()) else {
        return;
    };

    let keys: Vec<String> = deps.keys().cloned().collect();
    for key in keys {
        let Some(val) = deps.get(&key) else { continue };
        if !is_workspace_inherited(val) {
            continue;
        }

        // Capture member-level overrides before replacing.
        let member_table = val.as_table().cloned().unwrap_or_default();

        let Some(ws_entry) = ws_deps.get(&key) else {
            // No workspace definition — leave as-is (will error later, but
            // that's the user's problem, not ours to silently swallow).
            continue;
        };

        // Build the resolved dep value.
        let mut resolved = match ws_entry {
            toml::Value::String(version) => {
                // Simple form: `dep = "version"` in workspace.
                let mut t = toml::Table::new();
                t.insert("version".to_string(), toml::Value::String(version.clone()));
                t
            }
            toml::Value::Table(t) => t.clone(),
            _ => continue,
        };

        // Remove `workspace = true` if it somehow ended up in the resolved table.
        resolved.remove("workspace");

        // Strip `path` — intra-workspace paths are meaningless in vendor.
        resolved.remove("path");

        // Apply member-level overrides.
        apply_member_overrides(&mut resolved, &member_table);

        deps.insert(key, toml::Value::Table(resolved));
    }
}

/// Merge member-level overrides onto a resolved workspace dependency entry.
///
/// - `features` is additive (workspace features + member features).
/// - `default-features` and `optional` override the workspace value.
fn apply_member_overrides(resolved: &mut toml::Table, member: &toml::Table) {
    // `features` — merge additively, dedup.
    if let Some(member_feats) = member.get("features").and_then(|v| v.as_array()) {
        let mut all_feats: Vec<toml::Value> = resolved
            .get("features")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for f in member_feats {
            if !all_feats.contains(f) {
                all_feats.push(f.clone());
            }
        }
        resolved.insert("features".to_string(), toml::Value::Array(all_feats));
    }

    // `default-features` — member overrides workspace.
    if let Some(df) = member.get("default-features") {
        resolved.insert("default-features".to_string(), df.clone());
    }

    // `optional` — member overrides workspace.
    if let Some(opt) = member.get("optional") {
        resolved.insert("optional".to_string(), opt.clone());
    }
}

// ---------------------------------------------------------------------------
// Semver version resolution
// ---------------------------------------------------------------------------

/// In-memory + persistent cache for crates.io version listings.
///
/// Optionally backed by `build/vendor-version-cache.json` to avoid redundant
/// crates.io API queries across runs. Entries older than 24 hours are discarded.
pub struct VersionCache {
    entries: std::collections::HashMap<String, Vec<CrateVersionEntry>>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CrateVersionEntry {
    num: String,
    yanked: bool,
}

/// On-disk representation of the version cache.
#[derive(serde::Serialize, serde::Deserialize)]
struct PersistedVersionCache {
    /// Unix epoch seconds when this cache was written.
    written_at: u64,
    entries: std::collections::HashMap<String, Vec<CrateVersionEntry>>,
}

/// Maximum age of the persisted cache before it is discarded (24 hours).
const VERSION_CACHE_MAX_AGE_SECS: u64 = 24 * 60 * 60;

impl VersionCache {
    pub fn new() -> Self {
        Self { entries: std::collections::HashMap::new() }
    }

    /// Load a persisted version cache from `<build_dir>/vendor-version-cache.json`.
    ///
    /// Returns a fresh empty cache if the file is missing, unreadable, or older
    /// than [`VERSION_CACHE_MAX_AGE_SECS`].
    pub fn load(build_dir: &Path) -> Self {
        let path = build_dir.join("vendor-version-cache.json");
        let Ok(data) = std::fs::read_to_string(&path) else {
            return Self::new();
        };
        let Ok(persisted) = serde_json::from_str::<PersistedVersionCache>(&data) else {
            return Self::new();
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        if now.saturating_sub(persisted.written_at) > VERSION_CACHE_MAX_AGE_SECS {
            return Self::new();
        }

        crate::verbose::vprintln!(
            "  loaded vendor version cache ({} entries)",
            persisted.entries.len()
        );
        Self { entries: persisted.entries }
    }

    /// Persist the current cache to `<build_dir>/vendor-version-cache.json`.
    pub fn save(&self, build_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(build_dir)?;
        let path = build_dir.join("vendor-version-cache.json");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let persisted = PersistedVersionCache {
            written_at: now,
            entries: self.entries.clone(),
        };

        let json = serde_json::to_string_pretty(&persisted)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}

/// Fetch the version listing for a single crate from crates.io.
///
/// Returns `(crate_name, version_entries)` on success. This function is
/// self-contained and safe to call from worker threads.
fn fetch_version_listing(name: &str) -> Result<(String, Vec<CrateVersionEntry>)> {
    let url = format!("https://crates.io/api/v1/crates/{name}");
    let output = std::process::Command::new("curl")
        .args(["-sSfL", "-H", "User-Agent: gluon-build-system", &url])
        .output()
        .context("failed to run curl")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to query crates.io for '{name}': {stderr}");
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parsing crates.io response for '{name}'"))?;

    let versions = json.get("versions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow::anyhow!("no 'versions' array in crates.io response for '{name}'"))?;

    let entries: Vec<CrateVersionEntry> = versions.iter().filter_map(|v| {
        let num = v.get("num")?.as_str()?.to_string();
        let yanked = v.get("yanked").and_then(|y| y.as_bool()).unwrap_or(false);
        Some(CrateVersionEntry { num, yanked })
    }).collect();

    Ok((name.to_string(), entries))
}

/// Ensure crate version listings are present in the cache, fetching from
/// crates.io if needed.
fn ensure_versions_cached(name: &str, cache: &mut VersionCache) -> Result<()> {
    if cache.entries.contains_key(name) {
        return Ok(());
    }

    let (crate_name, entries) = fetch_version_listing(name)?;
    cache.entries.insert(crate_name, entries);
    Ok(())
}

/// Prefetch version listings for multiple crates in parallel.
///
/// Filters out names already present in the cache, then spawns parallel
/// curl calls in batches. Results are merged into the cache.
pub fn prefetch_versions(
    names: &[String],
    cache: &mut VersionCache,
    batch_size: usize,
) -> Result<()> {
    let missing: Vec<&String> = names.iter()
        .filter(|n| !cache.entries.contains_key(n.as_str()))
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    crate::verbose::vprintln!(
        "  prefetching version listings for {} crates ({} batches of {})",
        missing.len(),
        (missing.len() + batch_size - 1) / batch_size,
        batch_size,
    );

    for chunk in missing.chunks(batch_size) {
        let results: Vec<Result<(String, Vec<CrateVersionEntry>)>> =
            std::thread::scope(|s| {
                let handles: Vec<_> = chunk.iter().map(|name| {
                    s.spawn(move || fetch_version_listing(name))
                }).collect();

                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });

        for result in results {
            let (name, entries) = result?;
            cache.entries.insert(name, entries);
        }
    }

    Ok(())
}

/// Resolve a version string to an exact version.
///
/// If `version_str` is already an exact semver version, returns it as-is.
/// Otherwise, treats it as a `VersionReq` and queries crates.io for the
/// newest non-yanked release that matches.
pub fn resolve_version(
    name: &str,
    version_str: &str,
    cache: &mut VersionCache,
) -> Result<String> {
    // If it parses as an exact version, return immediately.
    if semver::Version::parse(version_str).is_ok() {
        return Ok(version_str.to_string());
    }

    // Parse as a version requirement (e.g. "0.4", "^1.2", ">=0.3, <0.5").
    let req: semver::VersionReq = version_str.parse()
        .with_context(|| format!("invalid version requirement '{version_str}' for '{name}'"))?;

    ensure_versions_cached(name, cache)?;
    let versions = &cache.entries[name];

    // Find the newest non-yanked version that matches the requirement.
    let mut best: Option<(semver::Version, &str)> = None;
    for entry in versions {
        if entry.yanked {
            continue;
        }
        let Ok(ver) = semver::Version::parse(&entry.num) else {
            continue;
        };
        if req.matches(&ver) {
            if best.as_ref().map_or(true, |(b, _)| ver > *b) {
                best = Some((ver, &entry.num));
            }
        }
    }

    match best {
        Some((_, num)) => Ok(num.to_string()),
        None => bail!(
            "no non-yanked version of '{name}' matches requirement '{version_str}'"
        ),
    }
}

// ---------------------------------------------------------------------------
// Fetching
// ---------------------------------------------------------------------------

/// Fetch a crate from crates.io by downloading and extracting the .crate tarball.
pub fn fetch_crates_io(name: &str, version: &str, vendor_dir: &Path) -> Result<std::path::PathBuf> {
    let dest = vendor_dir.join(format!("{name}-{version}"));
    if dest.exists() {
        if dest.join("Cargo.toml").exists() {
            return Ok(dest);
        }
        println!("  Removing corrupt vendor directory: {}", dest.display());
        std::fs::remove_dir_all(&dest)
            .with_context(|| format!("removing corrupt vendor dir {}", dest.display()))?;
    }

    let url = format!(
        "https://crates.io/api/v1/crates/{name}/{version}/download"
    );
    println!("  Downloading {name} v{version} from crates.io...");

    // Download the .crate tarball.
    let output = std::process::Command::new("curl")
        .args(["-sSfL", &url])
        .output()
        .context("failed to run curl")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("failed to download {name} v{version}: {stderr}");
    }

    // Extract the gzipped tarball.
    let decoder = flate2::read::GzDecoder::new(&output.stdout[..]);
    let mut archive = tar::Archive::new(decoder);

    // .crate tarballs contain a top-level directory named {name}-{version}/.
    std::fs::create_dir_all(vendor_dir)?;
    archive.unpack(vendor_dir)
        .with_context(|| format!("extracting {name}-{version}.crate"))?;

    if !dest.exists() {
        bail!(
            "extracted archive but expected directory {} not found",
            dest.display()
        );
    }

    Ok(dest)
}

/// Fetch a crate from a git repository.
pub fn fetch_git(
    name: &str,
    url: &str,
    reference: &str,
    vendor_dir: &Path,
) -> Result<std::path::PathBuf> {
    let short_ref = if reference.len() > 8 { &reference[..8] } else { reference };
    let dest = vendor_dir.join(format!("{name}-{short_ref}"));
    if dest.exists() {
        if dest.join("Cargo.toml").exists() {
            return Ok(dest);
        }
        println!("  Removing corrupt vendor directory: {}", dest.display());
        std::fs::remove_dir_all(&dest)
            .with_context(|| format!("removing corrupt vendor dir {}", dest.display()))?;
    }

    println!("  Cloning {name} from {url} (ref: {reference})...");

    let tmp_dir = vendor_dir.join(format!(".tmp-{name}"));
    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)?;
    }

    // Clone the repository.
    let mut cmd = std::process::Command::new("git");
    cmd.args(["clone", "--depth", "1"]);

    // For branch/tag, use --branch.
    if reference != "HEAD" {
        cmd.args(["--branch", reference]);
    }

    cmd.args([url, tmp_dir.to_str().unwrap()]);

    let output = cmd.output().context("failed to run git clone")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // If --branch failed (for a commit rev), do a full clone + checkout.
        let mut cmd2 = std::process::Command::new("git");
        cmd2.args(["clone", url, tmp_dir.to_str().unwrap()]);
        let output2 = cmd2.output().context("failed to run git clone")?;
        if !output2.status.success() {
            bail!("failed to clone {url}: {stderr}");
        }

        let checkout = std::process::Command::new("git")
            .args(["-C", tmp_dir.to_str().unwrap(), "checkout", reference])
            .output()
            .context("failed to run git checkout")?;
        if !checkout.status.success() {
            let stderr2 = String::from_utf8_lossy(&checkout.stderr);
            bail!("failed to checkout {reference}: {stderr2}");
        }
    }

    // Check if this is a workspace — look for the crate as a member.
    let workspace_cargo = tmp_dir.join("Cargo.toml");
    if workspace_cargo.exists() {
        let content = std::fs::read_to_string(&workspace_cargo)?;
        if let Ok(doc) = content.parse::<toml::Table>() {
            // Check if the root package matches.
            let root_name = doc.get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str());

            if root_name == Some(name) {
                // Root package is what we want — move it directly.
                std::fs::rename(&tmp_dir, &dest)?;
                return Ok(dest);
            }

            // Look for workspace members.
            if let Some(workspace) = doc.get("workspace").and_then(|w| w.as_table()) {
                if let Some(members) = workspace.get("members").and_then(|m| m.as_array()) {
                    for member in members {
                        if let Some(member_path) = member.as_str() {
                            let member_dir = tmp_dir.join(member_path);
                            let member_cargo = member_dir.join("Cargo.toml");
                            if member_cargo.exists() {
                                if let Ok(mcontent) = std::fs::read_to_string(&member_cargo) {
                                    if let Ok(mdoc) = mcontent.parse::<toml::Table>() {
                                        let member_name = mdoc.get("package")
                                            .and_then(|p| p.get("name"))
                                            .and_then(|n| n.as_str());
                                        if member_name == Some(name) {
                                            // Copy just this member.
                                            copy_dir_recursive(&member_dir, &dest)?;

                                            // Resolve workspace = true references
                                            // so the vendored crate is self-contained.
                                            let ws_meta = parse_workspace_metadata(&doc);
                                            let dest_cargo = dest.join("Cargo.toml");
                                            if let Ok(raw) = std::fs::read_to_string(&dest_cargo) {
                                                if let Ok(mut member_doc) = raw.parse::<toml::Table>() {
                                                    resolve_workspace_references(&mut member_doc, &ws_meta);
                                                    let normalized = toml::to_string_pretty(&member_doc)
                                                        .expect("resolved Cargo.toml should serialize");
                                                    let _ = std::fs::write(&dest_cargo, normalized);
                                                }
                                            }

                                            std::fs::remove_dir_all(&tmp_dir)?;
                                            return Ok(dest);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: move the entire clone.
    std::fs::rename(&tmp_dir, &dest)?;
    Ok(dest)
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            // Skip .git directories.
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-vendor check
// ---------------------------------------------------------------------------

/// Remove vendor directories that are corrupt (exist but have no Cargo.toml).
///
/// Returns the number of directories removed.
pub fn cleanup_corrupt_vendor_dirs(vendor_dir: &Path) -> Result<usize> {
    let mut removed = 0;
    if let Ok(entries) = std::fs::read_dir(vendor_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && !path.join("Cargo.toml").exists()
                // Skip hidden dirs (e.g. .tmp-*)
                && !entry.file_name().to_string_lossy().starts_with('.')
            {
                println!("  Removing corrupt vendor directory: {}", path.display());
                std::fs::remove_dir_all(&path)
                    .with_context(|| format!("removing corrupt vendor dir {}", path.display()))?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Check if vendoring is needed and perform it if so.
///
/// Returns `Ok(true)` if vendoring was performed, `Ok(false)` if everything
/// was already up to date.
pub fn ensure_vendored(
    root: &Path,
    dependencies: &std::collections::BTreeMap<String, crate::model::ExternalDepDef>,
    force: bool,
    jobs: usize,
) -> Result<bool> {
    let vendor_dir = root.join("vendor");
    let lock_path = root.join("gluon.lock");

    // Clean up corrupt vendor dirs before checking.
    if vendor_dir.exists() {
        let cleaned = cleanup_corrupt_vendor_dirs(&vendor_dir)?;
        if cleaned > 0 {
            println!("  Cleaned up {cleaned} corrupt vendor director{}", if cleaned == 1 { "y" } else { "ies" });
        }
    }

    // Quick check: if lock file exists and all root deps have Cargo.toml, skip.
    if !force && lock_path.exists() {
        let all_present = dependencies.iter().all(|(name, _)| {
            let dest = find_vendor_dir(name, None, &vendor_dir);
            dest.join("Cargo.toml").exists()
        });
        if all_present {
            return Ok(false);
        }
    }

    println!("Auto-vendoring missing dependencies...");

    let build_dir = root.join("build");
    let mut version_cache = if force {
        VersionCache::new()
    } else {
        VersionCache::load(&build_dir)
    };

    // Prefetch version listings for crates.io deps.
    let prefetch_batch = if jobs > 0 { jobs } else { 8 };
    let crates_io_names: Vec<String> = dependencies.iter()
        .filter_map(|(name, dep)| match &dep.source {
            crate::model::DepSource::CratesIo { version } if !version.is_empty() => {
                if semver::Version::parse(version).is_err() {
                    Some(name.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    if !crates_io_names.is_empty() {
        prefetch_versions(&crates_io_names, &mut version_cache, prefetch_batch)?;
    }

    // Fetch missing root deps.
    for (name, dep) in dependencies {
        match &dep.source {
            crate::model::DepSource::CratesIo { version } => {
                if version.is_empty() {
                    anyhow::bail!("dependency '{name}' has no version specified");
                }
                let resolved_version = resolve_version(name, version, &mut version_cache)?;
                let dest = find_vendor_dir(name, Some(&resolved_version), &vendor_dir);
                if !dest.join("Cargo.toml").exists() {
                    if dest.exists() { std::fs::remove_dir_all(&dest)?; }
                    fetch_crates_io(name, &resolved_version, &vendor_dir)?;
                }
            }
            crate::model::DepSource::Git { url, reference } => {
                let ref_str = match reference {
                    crate::model::GitRef::Rev(r) => r.clone(),
                    crate::model::GitRef::Tag(t) => t.clone(),
                    crate::model::GitRef::Branch(b) => b.clone(),
                    crate::model::GitRef::Default => "HEAD".into(),
                };
                let dest = find_vendor_dir(name, None, &vendor_dir);
                if !dest.join("Cargo.toml").exists() {
                    if dest.exists() { std::fs::remove_dir_all(&dest)?; }
                    fetch_git(name, url, &ref_str, &vendor_dir)?;
                }
            }
            crate::model::DepSource::Path { .. } => {}
        }
    }

    // Resolve transitive dependencies iteratively.
    let max_iterations = 10;
    let fetch_batch = if jobs > 0 { jobs } else { 12 };
    let locked_versions = load_locked_versions(&lock_path);
    for iteration in 1..=max_iterations {
        let resolved = resolve_transitive(dependencies, &vendor_dir, &mut version_cache, &locked_versions)?;

        let to_fetch: Vec<&ResolvedDep> = resolved.iter()
            .filter(|dep| {
                let vendor_path = find_vendor_dir(&dep.name, Some(&dep.version), &vendor_dir);
                !vendor_path.join("Cargo.toml").exists()
                    && !matches!(dep.source, ResolvedSource::Path { .. })
            })
            .collect();

        if to_fetch.is_empty() {
            // All deps present — write lock file.
            let lock = build_lock_file(&resolved, &vendor_dir)?;
            write_lock_file(&lock_path, &lock)?;

            if let Err(e) = version_cache.save(&build_dir) {
                crate::verbose::vprintln!("  warning: failed to save version cache: {e}");
            }

            return Ok(true);
        }

        if iteration == max_iterations {
            anyhow::bail!("transitive resolution did not converge after {max_iterations} iterations");
        }

        // Fetch missing transitive deps.
        for chunk in to_fetch.chunks(fetch_batch) {
            let results: Vec<Result<std::path::PathBuf>> = std::thread::scope(|s| {
                let handles: Vec<_> = chunk.iter().map(|dep| {
                    let vdir = &vendor_dir;
                    s.spawn(move || -> Result<std::path::PathBuf> {
                        match &dep.source {
                            ResolvedSource::CratesIo => {
                                fetch_crates_io(&dep.name, &dep.version, vdir)
                            }
                            ResolvedSource::Git { url, reference } => {
                                fetch_git(&dep.name, url, reference, vdir)
                            }
                            ResolvedSource::Path { .. } => {
                                Ok(vdir.join(&dep.name))
                            }
                        }
                    })
                }).collect();
                handles.into_iter().map(|h| h.join().unwrap()).collect()
            });

            for result in results {
                result?;
            }
        }
    }

    Ok(true)
}

// ---------------------------------------------------------------------------
// Lock file
// ---------------------------------------------------------------------------

/// Lock file data structure.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LockFile {
    pub version: u32,
    #[serde(rename = "package")]
    pub packages: Vec<LockPackage>,
}

/// A locked package entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LockPackage {
    pub name: String,
    pub version: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<Vec<String>>,
}

/// Read a lock file from disk.
pub fn read_lock_file(path: &Path) -> Result<Option<LockFile>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let lock: LockFile = toml::from_str(&content)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(lock))
}

/// Load locked versions from a lock file, returning a name → version map.
///
/// Returns an empty map if the lock file doesn't exist or can't be read.
pub fn load_locked_versions(lock_path: &Path) -> HashMap<String, String> {
    let Ok(Some(lock)) = read_lock_file(lock_path) else {
        return HashMap::new();
    };
    lock.packages.into_iter()
        .map(|pkg| (pkg.name, pkg.version))
        .collect()
}

/// Write a lock file to disk.
pub fn write_lock_file(path: &Path, lock: &LockFile) -> Result<()> {
    let header = "# This file is auto-generated by `gluon vendor`. Do not edit.\n\n";
    let content = toml::to_string_pretty(lock)
        .context("serializing lock file")?;
    std::fs::write(path, format!("{header}{content}"))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Compute a SHA-256 checksum of a directory's contents (deterministic).
pub fn dir_checksum(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    let mut entries: Vec<_> = walkdir::WalkDir::new(path)
        .sort_by_file_name()
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .collect();
    entries.sort_by(|a, b| a.path().cmp(b.path()));

    for entry in entries {
        // Hash the relative path.
        let rel = entry.path().strip_prefix(path).unwrap_or(entry.path());
        hasher.update(rel.to_string_lossy().as_bytes());
        // Hash the file contents.
        let contents = std::fs::read(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?;
        hasher.update(&contents);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

/// Build a lock file from resolved dependencies.
pub fn build_lock_file(
    resolved: &[ResolvedDep],
    vendor_dir: &Path,
) -> Result<LockFile> {
    let mut packages = Vec::new();

    for dep in resolved {
        let source = match &dep.source {
            ResolvedSource::CratesIo => "crates-io".to_string(),
            ResolvedSource::Git { url, reference } => format!("git+{url}#{reference}"),
            ResolvedSource::Path { path } => format!("path+{path}"),
        };

        let vendor_path = find_vendor_dir(&dep.name, Some(&dep.version), vendor_dir);
        let checksum = if vendor_path.exists() {
            Some(dir_checksum(&vendor_path)?)
        } else {
            None
        };

        // Get dependencies from Cargo.toml.
        let dependencies = if vendor_path.join("Cargo.toml").exists() {
            let parsed = parse_cargo_toml(&vendor_path.join("Cargo.toml")).ok();
            parsed.map(|p| {
                let activated = compute_activated_features(
                    &dep.features,
                    &p.features,
                    &p.default_features,
                );
                p.dependencies.iter()
                    .filter(|d| {
                        if d.optional {
                            let dep_feat_key = format!("dep:{}", d.key);
                            activated.contains(&d.key.to_string()) || activated.contains(&dep_feat_key)
                        } else {
                            true
                        }
                    })
                    .map(|d| d.name.clone())
                    .collect::<Vec<_>>()
            }).filter(|d| !d.is_empty())
        } else {
            None
        };

        packages.push(LockPackage {
            name: dep.name.clone(),
            version: dep.version.clone(),
            source,
            checksum,
            dependencies,
        });
    }

    Ok(LockFile {
        version: 1,
        packages,
    })
}

// ---------------------------------------------------------------------------
// Auto-registration into BuildModel
// ---------------------------------------------------------------------------

/// Register all resolved vendored dependencies as `CrateDef` entries in the
/// build model, creating synthetic groups for vendored and host crates.
pub fn auto_register_dependencies(
    model: &mut crate::model::BuildModel,
    resolved: &[ResolvedDep],
    vendor_dir: &Path,
    default_target: &str,
) -> Result<()> {
    // Create synthetic groups if they don't exist.
    if !model.groups.contains_key("vendored") {
        model.groups.insert("vendored".into(), crate::model::GroupDef {
            name: "vendored".into(),
            target: default_target.into(),
            default_edition: "2021".into(),
            crates: Vec::new(),
            shared_flags: Vec::new(),
            is_project: false,
            config: false,
        });
    }
    if !model.groups.contains_key("host") {
        model.groups.insert("host".into(), crate::model::GroupDef {
            name: "host".into(),
            target: "host".into(),
            default_edition: "2021".into(),
            crates: Vec::new(),
            shared_flags: Vec::new(),
            is_project: false,
            config: false,
        });
    }

    // Build a lookup map from resolved deps for O(1) version lookups.
    let resolved_map: HashMap<&str, &ResolvedDep> = resolved.iter()
        .map(|d| (d.name.as_str(), d))
        .collect();

    for dep in resolved {
        // Skip if already registered (e.g. a project crate with the same name).
        if model.crates.contains_key(&dep.name) {
            continue;
        }

        let vendor_path = find_vendor_dir(&dep.name, Some(&dep.version), vendor_dir);
        let cargo_toml_path = vendor_path.join("Cargo.toml");
        if !cargo_toml_path.exists() {
            // Not yet vendored — skip registration for now.
            // `gluon vendor` will fetch missing crates, then a subsequent
            // load_model() call will register them.
            continue;
        }

        let parsed = parse_cargo_toml(&cargo_toml_path)?;

        let crate_type = match parsed.crate_type {
            CargoCrateType::ProcMacro => crate::model::CrateType::ProcMacro,
            CargoCrateType::Lib => crate::model::CrateType::Lib,
        };

        // Determine group: proc-macros and their deps go to host.
        let is_host = dep.is_proc_macro || is_transitive_proc_macro_dep(&dep.name, resolved);
        let (group_name, target) = if is_host {
            ("host".to_string(), "host".to_string())
        } else {
            ("vendored".to_string(), default_target.to_string())
        };

        // Build dep map: only include non-optional deps + feature-activated optional deps.
        let activated = compute_activated_features(
            &dep.features,
            &parsed.features,
            &parsed.default_features,
        );

        let mut deps = BTreeMap::new();
        for cargo_dep in &parsed.dependencies {
            if cargo_dep.optional {
                let dep_feat_key = format!("dep:{}", cargo_dep.key);
                if !activated.contains(&cargo_dep.key.to_string())
                    && !activated.contains(&dep_feat_key)
                {
                    continue;
                }
            }

            let extern_name = cargo_dep.key.replace('-', "_");
            // Look up the resolved version of this transitive dep.
            let dep_version = resolved_map
                .get(cargo_dep.name.as_str())
                .map(|r| r.version.clone())
                .filter(|v| !v.is_empty());
            deps.insert(
                extern_name.clone(),
                crate::model::DepDef {
                    extern_name,
                    crate_name: cargo_dep.name.clone(),
                    features: Vec::new(),
                    version: dep_version,
                },
            );
        }

        // Compute the relative path from project root to vendor dir.
        let rel_path = vendor_path
            .strip_prefix(vendor_dir.parent().unwrap_or(vendor_dir))
            .unwrap_or(&vendor_path)
            .to_string_lossy()
            .to_string();

        // Look up cfg_flags from the dependency declaration.
        let cfg_flags = model.dependencies.get(&dep.name)
            .map(|d| d.cfg_flags.clone())
            .unwrap_or_default();

        let crate_def = crate::model::CrateDef {
            name: dep.name.clone(),
            path: rel_path,
            edition: parsed.package.edition,
            crate_type,
            target,
            deps,
            dev_deps: BTreeMap::new(),
            features: dep.features.clone(),
            root: None,
            linker_script: None,
            group: Some(group_name.clone()),
            is_project_crate: false,
            cfg_flags,
            requires_config: Vec::new(),
        };

        model.crates.insert(dep.name.clone(), crate_def);

        // Add to the appropriate group.
        if let Some(group) = model.groups.get_mut(&group_name) {
            group.crates.push(dep.name.clone());
        }
    }

    Ok(())
}

/// Check if a dependency is transitively needed only by proc-macros.
fn is_transitive_proc_macro_dep(name: &str, _resolved: &[ResolvedDep]) -> bool {
    // A dep is a host dep if all resolved deps that depend on it are proc-macros
    // or are themselves host deps. Simple heuristic: check if any proc-macro
    // in the resolved set transitively depends on this crate.
    //
    // For now, use a simpler approach: known proc-macro ecosystem crates.
    matches!(name, "proc-macro2" | "quote" | "syn" | "unicode-ident")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("gluon_vendor_test_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // -----------------------------------------------------------------------
    // parse_cargo_toml_str tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_minimal_cargo_toml() {
        let content = r#"
[package]
name = "foo"
version = "1.0.0"
"#;
        let parsed = parse_cargo_toml_str(content, Path::new("test/Cargo.toml")).unwrap();
        assert_eq!(parsed.package.name, "foo");
        assert_eq!(parsed.package.version, "1.0.0");
        assert_eq!(parsed.package.edition, "2021");
        assert_eq!(parsed.crate_type, CargoCrateType::Lib);
        assert!(parsed.dependencies.is_empty());
        assert!(parsed.features.is_empty());
        assert!(parsed.default_features.is_empty());
        assert!(parsed.lib_name.is_none());
    }

    #[test]
    fn parse_proc_macro_crate() {
        let content = r#"
[package]
name = "my-derive"
version = "0.1.0"

[lib]
proc-macro = true
"#;
        let parsed = parse_cargo_toml_str(content, Path::new("test/Cargo.toml")).unwrap();
        assert_eq!(parsed.crate_type, CargoCrateType::ProcMacro);
    }

    #[test]
    fn parse_optional_dep() {
        let content = r#"
[package]
name = "bar"
version = "2.0.0"

[dependencies]
foo = { version = "1.0", optional = true }
"#;
        let parsed = parse_cargo_toml_str(content, Path::new("test/Cargo.toml")).unwrap();
        assert_eq!(parsed.dependencies.len(), 1);
        let dep = &parsed.dependencies[0];
        assert_eq!(dep.name, "foo");
        assert_eq!(dep.key, "foo");
        assert_eq!(dep.version.as_deref(), Some("1.0"));
        assert!(dep.optional);
        assert!(dep.default_features);
    }

    #[test]
    fn parse_features_section() {
        let content = r#"
[package]
name = "baz"
version = "0.1.0"

[features]
default = ["std"]
std = ["alloc"]
alloc = []
"#;
        let parsed = parse_cargo_toml_str(content, Path::new("test/Cargo.toml")).unwrap();
        assert_eq!(parsed.default_features, vec!["std".to_string()]);
        assert_eq!(
            parsed.features.get("std").unwrap(),
            &vec!["alloc".to_string()]
        );
        assert!(parsed.features.get("alloc").unwrap().is_empty());
        assert_eq!(
            parsed.features.get("default").unwrap(),
            &vec!["std".to_string()]
        );
    }

    #[test]
    fn parse_dep_with_package_rename() {
        let content = r#"
[package]
name = "consumer"
version = "0.1.0"

[dependencies]
my_foo = { package = "foo", version = "1.0" }
"#;
        let parsed = parse_cargo_toml_str(content, Path::new("test/Cargo.toml")).unwrap();
        assert_eq!(parsed.dependencies.len(), 1);
        let dep = &parsed.dependencies[0];
        assert_eq!(dep.key, "my_foo");
        assert_eq!(dep.name, "foo");
        assert_eq!(dep.version.as_deref(), Some("1.0"));
    }

    // -----------------------------------------------------------------------
    // compute_activated_features tests
    // -----------------------------------------------------------------------

    #[test]
    fn default_features_expanded() {
        let requested = vec!["__default__".to_string()];
        let mut feature_table = BTreeMap::new();
        feature_table.insert("std".to_string(), vec!["alloc".to_string()]);
        feature_table.insert("alloc".to_string(), vec![]);
        let default_features = vec!["std".to_string()];

        let activated = compute_activated_features(&requested, &feature_table, &default_features);

        assert!(activated.contains(&"std".to_string()));
        assert!(activated.contains(&"alloc".to_string()));
    }

    #[test]
    fn no_default_marker() {
        let requested = vec!["serde".to_string()];
        let mut feature_table = BTreeMap::new();
        feature_table.insert("default".to_string(), vec!["std".to_string()]);
        feature_table.insert("std".to_string(), vec![]);
        feature_table.insert("serde".to_string(), vec!["dep:serde".to_string()]);
        let default_features = vec!["std".to_string()];

        let activated = compute_activated_features(&requested, &feature_table, &default_features);

        assert!(activated.contains(&"serde".to_string()));
        assert!(activated.contains(&"dep:serde".to_string()));
        // No "__default__" was requested, so default features should NOT be activated.
        assert!(!activated.contains(&"std".to_string()));
    }

    #[test]
    fn transitive_features() {
        let requested = vec!["full".to_string()];
        let mut feature_table = BTreeMap::new();
        feature_table.insert("full".to_string(), vec!["a".to_string(), "b".to_string()]);
        feature_table.insert("a".to_string(), vec!["c".to_string()]);
        feature_table.insert("b".to_string(), vec![]);
        feature_table.insert("c".to_string(), vec![]);
        let default_features = vec![];

        let activated = compute_activated_features(&requested, &feature_table, &default_features);

        assert!(activated.contains(&"full".to_string()));
        assert!(activated.contains(&"a".to_string()));
        assert!(activated.contains(&"b".to_string()));
        assert!(activated.contains(&"c".to_string()));
    }

    #[test]
    fn duplicate_features_not_repeated() {
        let requested = vec!["a".to_string(), "a".to_string()];
        let feature_table = BTreeMap::new();
        let default_features = vec![];

        let activated = compute_activated_features(&requested, &feature_table, &default_features);

        let count = activated.iter().filter(|f| f.as_str() == "a").count();
        assert_eq!(count, 1, "feature 'a' should appear exactly once");
    }

    // -----------------------------------------------------------------------
    // is_workspace_inherited tests
    // -----------------------------------------------------------------------

    #[test]
    fn workspace_true() {
        let mut table = toml::Table::new();
        table.insert("workspace".to_string(), toml::Value::Boolean(true));
        let value = toml::Value::Table(table);
        assert!(is_workspace_inherited(&value));
    }

    #[test]
    fn workspace_false() {
        let mut table = toml::Table::new();
        table.insert("workspace".to_string(), toml::Value::Boolean(false));
        let value = toml::Value::Table(table);
        assert!(!is_workspace_inherited(&value));
    }

    #[test]
    fn not_a_table() {
        let value = toml::Value::String("1.0".to_string());
        assert!(!is_workspace_inherited(&value));
    }

    // -----------------------------------------------------------------------
    // find_vendor_dir tests
    // -----------------------------------------------------------------------

    /// Helper: create a fake Cargo.toml inside a directory.
    fn touch_cargo_toml(dir: &std::path::Path) {
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"fake\"\nversion = \"0.0.0\"\n",
        ).unwrap();
    }

    #[test]
    fn find_versioned_vendor_dir() {
        let dir = make_test_dir("find_versioned");
        let versioned = dir.join("foo-1.0.0");
        std::fs::create_dir_all(&versioned).unwrap();
        touch_cargo_toml(&versioned);

        let result = find_vendor_dir("foo", None, &dir);
        assert_eq!(result, versioned);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_unversioned_vendor_dir() {
        let dir = make_test_dir("find_unversioned");
        let unversioned = dir.join("foo");
        std::fs::create_dir_all(&unversioned).unwrap();

        let result = find_vendor_dir("foo", None, &dir);
        assert_eq!(result, unversioned);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn versioned_preferred_over_unversioned() {
        let dir = make_test_dir("find_versioned_preferred");
        let versioned = dir.join("foo-1.0.0");
        let unversioned = dir.join("foo");
        std::fs::create_dir_all(&versioned).unwrap();
        touch_cargo_toml(&versioned);
        std::fs::create_dir_all(&unversioned).unwrap();

        let result = find_vendor_dir("foo", None, &dir);
        assert_eq!(result, versioned);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_exact_version_match() {
        let dir = make_test_dir("find_exact_version");
        let v100 = dir.join("foo-1.0.0");
        let v104 = dir.join("foo-1.0.4");
        std::fs::create_dir_all(&v100).unwrap();
        std::fs::create_dir_all(&v104).unwrap();
        touch_cargo_toml(&v100);
        touch_cargo_toml(&v104);

        // With exact version, should get the exact match.
        let result = find_vendor_dir("foo", Some("1.0.0"), &dir);
        assert_eq!(result, v100);

        let result = find_vendor_dir("foo", Some("1.0.4"), &dir);
        assert_eq!(result, v104);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_skips_corrupt_vendor_dir() {
        let dir = make_test_dir("find_skips_corrupt");
        let corrupt = dir.join("foo-1.0.4");
        let valid = dir.join("foo-1.0.0");
        // Create corrupt dir (no Cargo.toml) and valid dir.
        std::fs::create_dir_all(&corrupt).unwrap();
        std::fs::create_dir_all(&valid).unwrap();
        touch_cargo_toml(&valid);

        // Without version hint, should skip corrupt and find valid.
        let result = find_vendor_dir("foo", None, &dir);
        assert_eq!(result, valid);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
