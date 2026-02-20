//! Dependency vendoring: Cargo.toml parsing, transitive resolution, fetching,
//! lock file management, and auto-registration into the build model.

use std::collections::BTreeMap;
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
/// vendored, with unified features.
pub fn resolve_transitive(
    roots: &BTreeMap<String, crate::model::ExternalDepDef>,
    vendor_dir: &Path,
    version_cache: &mut VersionCache,
) -> Result<Vec<ResolvedDep>> {
    let mut resolved: BTreeMap<String, ResolvedDep> = BTreeMap::new();
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
        });
    }

    // BFS resolution.
    while let Some(entry) = queue.pop_front() {
        if let Some(existing) = resolved.get_mut(&entry.name) {
            // Already resolved — unify features.
            let old_len = existing.features.len();
            for feat in &entry.requested_features {
                if !existing.features.contains(feat) {
                    existing.features.push(feat.clone());
                }
            }
            if existing.features.len() == old_len {
                // No new features to propagate.
                continue;
            }
            // Features changed — need to re-process this dep's transitive deps.
        } else {
            resolved.insert(entry.name.clone(), ResolvedDep {
                name: entry.name.clone(),
                version: entry.version.clone(),
                source: entry.source.clone(),
                features: entry.requested_features.clone(),
                is_proc_macro: false,
            });
        }

        // Find the vendored Cargo.toml to discover transitive deps.
        let vendor_path = find_vendor_dir(&entry.name, vendor_dir);
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
            if cargo_dep.default_features {
                trans_features.push("__default__".to_string());
            }
            trans_features.extend(cargo_dep.features.clone());

            // Propagate features from parent feature specs (e.g. "dep/feature").
            for feat_spec in &activated {
                if let Some(rest) = feat_spec.strip_prefix(&format!("{}/", cargo_dep.key)) {
                    if !trans_features.contains(&rest.to_string()) {
                        trans_features.push(rest.to_string());
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
                resolve_version(&cargo_dep.name, &raw_version, version_cache)?
            } else {
                raw_version
            };

            queue.push_back(QueueEntry {
                name: cargo_dep.name.clone(),
                version: resolved_version,
                source,
                requested_features: trans_features,
            });
        }
    }

    // Convert "__default__" markers into actual default features.
    for dep in resolved.values_mut() {
        let vendor_path = find_vendor_dir(&dep.name, vendor_dir);
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
                let mut seen = std::collections::HashSet::new();
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

    let mut seen = std::collections::HashSet::new();
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
/// Checks for `vendor/{name}-{version}/` first, then `vendor/{name}/`.
pub fn find_vendor_dir(name: &str, vendor_dir: &Path) -> std::path::PathBuf {
    // Check versioned directory pattern first.
    if let Ok(entries) = std::fs::read_dir(vendor_dir) {
        let prefix = format!("{name}-");
        for entry in entries.flatten() {
            let fname = entry.file_name();
            let fname_str = fname.to_string_lossy();
            if fname_str.starts_with(&prefix) && entry.path().is_dir() {
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

/// In-memory cache for crates.io version listings.
pub struct VersionCache {
    entries: std::collections::HashMap<String, Vec<CrateVersionEntry>>,
}

#[derive(Clone)]
struct CrateVersionEntry {
    num: String,
    yanked: bool,
}

impl VersionCache {
    pub fn new() -> Self {
        Self { entries: std::collections::HashMap::new() }
    }
}

/// Ensure crate version listings are present in the cache, fetching from
/// crates.io if needed.
fn ensure_versions_cached(name: &str, cache: &mut VersionCache) -> Result<()> {
    if cache.entries.contains_key(name) {
        return Ok(());
    }

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

    cache.entries.insert(name.to_string(), entries);
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
        return Ok(dest);
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
        return Ok(dest);
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

        let vendor_path = find_vendor_dir(&dep.name, vendor_dir);
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

    for dep in resolved {
        // Skip if already registered (e.g. a project crate with the same name).
        if model.crates.contains_key(&dep.name) {
            continue;
        }

        let vendor_path = find_vendor_dir(&dep.name, vendor_dir);
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
            deps.insert(
                extern_name.clone(),
                crate::model::DepDef {
                    extern_name,
                    crate_name: cargo_dep.name.clone(),
                    features: Vec::new(),
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
