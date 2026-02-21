//! Crate graph resolution from the build model.
//!
//! Converts model crate definitions into resolved crates, builds a dependency
//! graph, and produces a topologically sorted compilation order using Kahn's
//! algorithm.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::model::CrateType;

/// A resolved crate ready for compilation.
#[derive(Debug)]
pub struct ResolvedCrate {
    pub name: String,
    pub edition: String,
    pub crate_type: CrateType,
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
        let crate_type = def.crate_type;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BuildModel, CrateDef, CrateType, DepDef, GroupDef};
    use std::collections::BTreeMap;

    const GROUP: &str = "test-group";

    /// Build a minimal `BuildModel` from a list of (name, [dep_names]) pairs.
    ///
    /// All crates are `Lib` type with edition 2024 targeting `"host"`, placed in
    /// a single group called [`GROUP`].
    fn make_model(crates: &[(&str, &[&str])]) -> BuildModel {
        let mut model = BuildModel::default();
        model.project.name = "test".into();
        model.project.version = "0.1.0".into();

        let mut group = GroupDef::default();
        group.name = GROUP.into();

        for &(name, deps) in crates {
            group.crates.push(name.to_string());

            let mut dep_map = BTreeMap::new();
            for &dep in deps {
                dep_map.insert(
                    dep.to_string(),
                    DepDef {
                        extern_name: dep.to_string(),
                        crate_name: dep.to_string(),
                        features: vec![],
                    },
                );
            }

            model.crates.insert(
                name.to_string(),
                CrateDef {
                    name: name.to_string(),
                    path: format!("crates/{name}"),
                    edition: "2024".into(),
                    crate_type: CrateType::Lib,
                    target: "host".into(),
                    deps: dep_map,
                    dev_deps: BTreeMap::new(),
                    features: vec![],
                    root: None,
                    linker_script: None,
                    group: Some(GROUP.into()),
                    is_project_crate: true,
                    cfg_flags: vec![],
                    requires_config: vec![],
                },
            );
        }

        model.groups.insert(GROUP.into(), group);
        model
    }

    /// Helper: resolve the test group and return names in sorted order.
    fn resolve_names(model: &BuildModel) -> Result<Vec<String>> {
        let root = Path::new("/fake/root");
        let sysroot = Path::new("/fake/sysroot");
        let resolved = resolve_group_from_model(model, GROUP, root, sysroot)?;
        Ok(resolved.iter().map(|c| c.name.clone()).collect())
    }

    /// Helper: return the position of `name` in the resolved order.
    fn position_of(names: &[String], name: &str) -> usize {
        names.iter().position(|n| n == name)
            .unwrap_or_else(|| panic!("crate '{name}' not found in resolved list"))
    }

    // ---- Test cases ----

    #[test]
    fn single_crate_no_deps() {
        let model = make_model(&[("foo", &[])]);
        let names = resolve_names(&model).expect("resolution should succeed");
        assert_eq!(names, vec!["foo"]);
    }

    #[test]
    fn linear_chain_sorted_correctly() {
        // A depends on B, B depends on C. Expected order: C, B, A.
        let model = make_model(&[
            ("a", &["b"]),
            ("b", &["c"]),
            ("c", &[]),
        ]);
        let names = resolve_names(&model).expect("resolution should succeed");
        assert_eq!(names.len(), 3);
        // C must come before B, B must come before A.
        assert!(
            position_of(&names, "c") < position_of(&names, "b"),
            "c must precede b, got: {names:?}",
        );
        assert!(
            position_of(&names, "b") < position_of(&names, "a"),
            "b must precede a, got: {names:?}",
        );
    }

    #[test]
    fn diamond_dependency() {
        // A depends on B and C; both B and C depend on D.
        let model = make_model(&[
            ("a", &["b", "c"]),
            ("b", &["d"]),
            ("c", &["d"]),
            ("d", &[]),
        ]);
        let names = resolve_names(&model).expect("resolution should succeed");
        assert_eq!(names.len(), 4);

        let pos_d = position_of(&names, "d");
        let pos_b = position_of(&names, "b");
        let pos_c = position_of(&names, "c");
        let pos_a = position_of(&names, "a");

        // D must come before both B and C, which must come before A.
        assert!(pos_d < pos_b, "d must precede b, got: {names:?}");
        assert!(pos_d < pos_c, "d must precede c, got: {names:?}");
        assert!(pos_b < pos_a, "b must precede a, got: {names:?}");
        assert!(pos_c < pos_a, "c must precede a, got: {names:?}");
    }

    #[test]
    fn independent_crates() {
        let model = make_model(&[("alpha", &[]), ("beta", &[])]);
        let names = resolve_names(&model).expect("resolution should succeed");
        assert_eq!(names.len(), 2);
        // Both must be present; order between them is unspecified.
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
    }

    #[test]
    fn cycle_detection() {
        // A depends on B, B depends on A.
        let model = make_model(&[("a", &["b"]), ("b", &["a"])]);
        let err = resolve_names(&model).expect_err("cycle should produce an error");
        let msg = err.to_string();
        assert!(
            msg.contains("cycle"),
            "error message should mention 'cycle', got: {msg}",
        );
    }

    #[test]
    fn missing_group_returns_error() {
        let model = BuildModel::default();
        let root = Path::new("/fake/root");
        let sysroot = Path::new("/fake/sysroot");
        let err = resolve_group_from_model(&model, "nonexistent", root, sysroot)
            .expect_err("missing group should produce an error");
        let msg = err.to_string();
        assert!(
            msg.contains("not found"),
            "error message should mention 'not found', got: {msg}",
        );
    }

    #[test]
    fn resolved_crate_fields_are_correct() {
        let model = make_model(&[("foo", &[])]);
        let root = Path::new("/fake/root");
        let sysroot = Path::new("/fake/sysroot");
        let resolved = resolve_group_from_model(&model, GROUP, root, sysroot)
            .expect("resolution should succeed");

        assert_eq!(resolved.len(), 1);
        let rc = &resolved[0];
        assert_eq!(rc.name, "foo");
        assert_eq!(rc.edition, "2024");
        assert_eq!(rc.crate_type, CrateType::Lib);
        assert_eq!(rc.target, "host");
        assert!(rc.deps.is_empty());
        assert!(rc.features.is_empty());
        assert_eq!(rc.root_file, PathBuf::from("/fake/root/crates/foo/src/lib.rs"));
        assert!(rc.linker_script.is_none());
        assert!(rc.is_project_crate);
        assert!(rc.cfg_flags.is_empty());
    }

    #[test]
    fn resolved_deps_are_populated() {
        let model = make_model(&[("app", &["lib"]), ("lib", &[])]);
        let root = Path::new("/fake/root");
        let sysroot = Path::new("/fake/sysroot");
        let resolved = resolve_group_from_model(&model, GROUP, root, sysroot)
            .expect("resolution should succeed");

        // "app" should have one resolved dependency pointing to "lib".
        let app = resolved.iter().find(|c| c.name == "app")
            .expect("app crate should be present");
        assert_eq!(app.deps.len(), 1);
        assert_eq!(app.deps[0].extern_name, "lib");
        assert_eq!(app.deps[0].crate_name, "lib");
    }

    #[test]
    fn external_deps_do_not_affect_in_degree() {
        // "a" depends on "ext" which is NOT in the group. The external dep
        // should be ignored for topological ordering but still appear in
        // the resolved deps list.
        let mut model = make_model(&[("a", &["ext"])]);
        // "ext" is intentionally not added to the group or model crates.
        // But DepDef references it. Since resolve_group_from_model filters
        // by name_set membership, "ext" should be skipped in graph building.
        // However, the model needs "a" to list it as a dep. Let's add ext
        // as a crate in the model (but NOT in the group) to match a real
        // scenario where a crate depends on something outside its group.
        model.crates.insert(
            "ext".to_string(),
            CrateDef {
                name: "ext".into(),
                path: "vendor/ext".into(),
                edition: "2024".into(),
                crate_type: CrateType::Lib,
                target: "host".into(),
                deps: BTreeMap::new(),
                dev_deps: BTreeMap::new(),
                features: vec![],
                root: None,
                linker_script: None,
                group: None,
                is_project_crate: false,
                cfg_flags: vec![],
                requires_config: vec![],
            },
        );

        let names = resolve_names(&model).expect("resolution should succeed");
        // Only "a" is in the group.
        assert_eq!(names, vec!["a"]);
    }

    #[test]
    fn three_node_cycle_detected() {
        // A -> B -> C -> A: a three-node cycle.
        let model = make_model(&[
            ("a", &["b"]),
            ("b", &["c"]),
            ("c", &["a"]),
        ]);
        let err = resolve_names(&model).expect_err("cycle should produce an error");
        let msg = err.to_string();
        assert!(
            msg.contains("cycle"),
            "error message should mention 'cycle', got: {msg}",
        );
    }
}
