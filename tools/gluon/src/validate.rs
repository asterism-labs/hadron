//! Post-evaluation validation of the [`BuildModel`].
//!
//! Checks referential integrity (all names resolve), constraint validity
//! (config option ranges, profile inheritance cycles), and pipeline
//! consistency (stages reference existing groups, rules exist).

use std::collections::{BTreeSet, HashSet};

use anyhow::{Result, bail, ensure};

use crate::model::{BuildModel, ConfigType, ConfigValue, DepSource, PipelineStep};

/// Validate a fully populated build model.
///
/// This should be called after Rhai evaluation and before resolution.
pub fn validate_model(model: &BuildModel) -> Result<()> {
    validate_project(model)?;
    validate_targets(model)?;
    validate_config_options(model)?;
    validate_profiles(model)?;
    validate_groups(model)?;
    validate_crates(model)?;
    validate_rules(model)?;
    validate_pipeline(model)?;
    validate_tests(model)?;
    validate_dependencies(model)?;
    Ok(())
}

fn validate_project(model: &BuildModel) -> Result<()> {
    ensure!(!model.project.name.is_empty(), "project name is required");
    ensure!(
        !model.project.version.is_empty(),
        "project version is required"
    );
    Ok(())
}

fn validate_targets(model: &BuildModel) -> Result<()> {
    for (name, target) in &model.targets {
        ensure!(
            !target.spec.is_empty(),
            "target '{name}' has no spec path"
        );
    }
    Ok(())
}

fn validate_config_options(model: &BuildModel) -> Result<()> {
    for (name, opt) in &model.config_options {
        // Default value type must match declared type.
        match (&opt.ty, &opt.default) {
            (ConfigType::Bool, ConfigValue::Bool(_)) => {}
            (ConfigType::U32, ConfigValue::U32(_)) => {}
            (ConfigType::U64, ConfigValue::U64(_)) => {}
            (ConfigType::Str, ConfigValue::Str(_)) => {}
            (ConfigType::Choice, ConfigValue::Choice(_)) => {}
            (ConfigType::List, ConfigValue::List(_)) => {}
            // Group uses a sentinel default (Bool(false)), skip check.
            (ConfigType::Group, _) => {}
            _ => bail!(
                "config option '{name}' default value type does not match declared type {:?}",
                opt.ty
            ),
        }

        // Range constraints only on numeric types.
        if let Some((min, max)) = opt.range {
            ensure!(
                matches!(opt.ty, ConfigType::U32 | ConfigType::U64),
                "config option '{name}' has range constraint but is not numeric"
            );
            ensure!(
                min <= max,
                "config option '{name}' range min ({min}) > max ({max})"
            );
        }

        // Choices: required for Choice type, allowed for Str type.
        if opt.ty == ConfigType::Choice {
            let choices = opt.choices.as_ref();
            ensure!(
                choices.is_some_and(|c| !c.is_empty()),
                "config option '{name}' is Choice type but has no variants"
            );
            // Default must be one of the variants.
            if let ConfigValue::Choice(ref default) = opt.default {
                ensure!(
                    choices.unwrap().contains(default),
                    "config option '{name}' default '{default}' is not in choices: {:?}",
                    choices.unwrap()
                );
            }
        } else if let Some(ref choices) = opt.choices {
            ensure!(
                matches!(opt.ty, ConfigType::Str | ConfigType::Choice),
                "config option '{name}' has choices but is not a string or choice type"
            );
            ensure!(
                !choices.is_empty(),
                "config option '{name}' has empty choices"
            );
        }

        // Group must not have choices or range.
        if opt.ty == ConfigType::Group {
            ensure!(
                opt.choices.is_none(),
                "config option '{name}' is Group type but has choices"
            );
            ensure!(
                opt.range.is_none(),
                "config option '{name}' is Group type but has range"
            );
        }

        // depends_on references must exist.
        for dep in &opt.depends_on {
            ensure!(
                model.config_options.contains_key(dep),
                "config option '{name}' depends on '{dep}', which is not defined"
            );
        }

        // selects references must exist.
        for sel in &opt.selects {
            ensure!(
                model.config_options.contains_key(sel),
                "config option '{name}' selects '{sel}', which is not defined"
            );
        }

        // Dotted-key referential integrity: prefix must be a defined Group.
        if let Some(dot_pos) = name.find('.') {
            let prefix = &name[..dot_pos];
            ensure!(
                model.config_options.get(prefix).is_some_and(|o| o.ty == ConfigType::Group),
                "config option '{name}' has dotted key but '{prefix}' is not a defined Group"
            );
        }
    }

    Ok(())
}

fn validate_profiles(model: &BuildModel) -> Result<()> {
    ensure!(
        model.profiles.contains_key("default"),
        "a 'default' profile is required"
    );

    // Check for inheritance cycles.
    for name in model.profiles.keys() {
        let mut visited = BTreeSet::new();
        let mut current = name.as_str();
        loop {
            if !visited.insert(current.to_string()) {
                bail!("profile inheritance cycle detected involving '{name}'");
            }
            match model.profiles.get(current) {
                Some(profile) => match &profile.inherits {
                    Some(parent) => {
                        ensure!(
                            model.profiles.contains_key(parent.as_str()),
                            "profile '{current}' inherits from '{parent}', which is not defined"
                        );
                        current = parent;
                    }
                    None => break,
                },
                None => bail!("profile '{current}' not found"),
            }
        }
    }

    // Config overrides in profiles must reference existing options.
    for (pname, profile) in &model.profiles {
        for key in profile.config.keys() {
            ensure!(
                model.config_options.contains_key(key),
                "profile '{pname}' overrides config '{key}', which is not defined"
            );
        }
    }

    Ok(())
}

fn validate_groups(model: &BuildModel) -> Result<()> {
    for (gname, group) in &model.groups {
        // Validate target is "host" or a defined target.
        if group.target != "host" {
            ensure!(
                model.targets.contains_key(&group.target),
                "group '{gname}' references target '{}', which is not defined",
                group.target
            );
        }
        for crate_name in &group.crates {
            ensure!(
                model.crates.contains_key(crate_name),
                "group '{gname}' references crate '{crate_name}', which is not defined"
            );
        }
    }
    Ok(())
}

fn validate_crates(model: &BuildModel) -> Result<()> {
    for (name, krate) in &model.crates {
        ensure!(
            !krate.path.is_empty(),
            "crate '{name}' has no path"
        );

        // All dependency references must resolve to a project crate or external dependency.
        for (extern_name, dep) in &krate.deps {
            ensure!(
                model.crates.contains_key(&dep.crate_name)
                    || model.dependencies.contains_key(&dep.crate_name),
                "crate '{name}' depends on '{ext}' (crate '{dep_name}'), which is not defined",
                ext = extern_name,
                dep_name = dep.crate_name
            );
        }

        // All dev dependency references must resolve.
        for (extern_name, dep) in &krate.dev_deps {
            ensure!(
                model.crates.contains_key(&dep.crate_name)
                    || model.dependencies.contains_key(&dep.crate_name),
                "crate '{name}' dev-depends on '{ext}' (crate '{dep_name}'), which is not defined",
                ext = extern_name,
                dep_name = dep.crate_name
            );
        }

        // Validate per-crate linker_script path if set.
        if let Some(ref ls) = krate.linker_script {
            ensure!(
                !ls.is_empty(),
                "crate '{name}' has an empty linker_script path"
            );
        }
    }

    // Check for dependency cycles within each target group.
    let mut targets_seen: HashSet<&str> = HashSet::new();
    for (_, krate) in &model.crates {
        targets_seen.insert(&krate.target);
    }

    for target in &targets_seen {
        let target_crates: HashSet<&str> = model
            .crates
            .iter()
            .filter(|(_, c)| c.target == *target)
            .map(|(n, _)| n.as_str())
            .collect();

        // Build in-degree and dependents map for Kahn's algorithm.
        let mut in_degree: std::collections::HashMap<&str, usize> = target_crates
            .iter()
            .map(|&n| (n, 0))
            .collect();
        let mut dependents: std::collections::HashMap<&str, Vec<&str>> =
            std::collections::HashMap::new();

        for (name, krate) in &model.crates {
            if krate.target != *target {
                continue;
            }
            for dep in krate.deps.values() {
                let dep_name = dep.crate_name.as_str();
                if target_crates.contains(dep_name) {
                    *in_degree.entry(name.as_str()).or_insert(0) += 1;
                    dependents.entry(dep_name).or_default().push(name.as_str());
                }
            }
        }

        let mut queue: std::collections::VecDeque<&str> = in_degree
            .iter()
            .filter(|&(_, d)| *d == 0)
            .map(|(&n, _)| n)
            .collect();

        let mut count = 0;
        while let Some(node) = queue.pop_front() {
            count += 1;
            if let Some(deps) = dependents.get(node) {
                for dep in deps {
                    if let Some(d) = in_degree.get_mut(dep) {
                        *d -= 1;
                        if *d == 0 {
                            queue.push_back(dep);
                        }
                    }
                }
            }
        }

        ensure!(
            count == target_crates.len(),
            "dependency cycle detected among crates targeting '{target}'"
        );
    }

    Ok(())
}

fn validate_rules(model: &BuildModel) -> Result<()> {
    for (rname, rule) in &model.rules {
        // Input crate references must exist.
        for input in &rule.inputs {
            ensure!(
                model.crates.contains_key(input) || input.contains('*'),
                "rule '{rname}' input '{input}' is not a known crate"
            );
        }

        // depends_on references must be other rules.
        for dep in &rule.depends_on {
            ensure!(
                model.rules.contains_key(dep),
                "rule '{rname}' depends on rule '{dep}', which is not defined"
            );
        }
    }
    Ok(())
}

fn validate_pipeline(model: &BuildModel) -> Result<()> {
    ensure!(
        !model.pipeline.steps.is_empty(),
        "pipeline must have at least one step"
    );

    for step in &model.pipeline.steps {
        match step {
            PipelineStep::Stage { name, groups } => {
                for gname in groups {
                    ensure!(
                        model.groups.contains_key(gname),
                        "pipeline stage '{name}' references group '{gname}', which is not defined"
                    );
                }
            }
            PipelineStep::Barrier(_) => {}
            PipelineStep::Rule(rname) => {
                ensure!(
                    model.rules.contains_key(rname),
                    "pipeline references rule '{rname}', which is not defined"
                );
            }
        }
    }

    Ok(())
}

fn validate_tests(model: &BuildModel) -> Result<()> {
    for name in &model.tests.host_testable {
        ensure!(
            model.crates.contains_key(name),
            "tests.host_testable references crate '{name}', which is not defined"
        );
    }

    // Validate kernel test config consistency.
    if let Some(ref crate_name) = model.tests.kernel_tests_crate {
        ensure!(
            model.crates.contains_key(crate_name),
            "tests.kernel_tests_crate references crate '{crate_name}', which is not defined"
        );
        let krate = &model.crates[crate_name];
        ensure!(
            !krate.dev_deps.is_empty(),
            "tests.kernel_tests_crate '{crate_name}' has no dev_deps"
        );
        ensure!(
            model.tests.kernel_tests_dir.is_some(),
            "tests.kernel_tests_crate is set but kernel_tests_dir is not"
        );
        ensure!(
            model.tests.kernel_tests_linker_script.is_some(),
            "tests.kernel_tests_crate is set but kernel_tests_linker_script is not"
        );
    }

    Ok(())
}

fn validate_dependencies(model: &BuildModel) -> Result<()> {
    for (name, dep) in &model.dependencies {
        ensure!(
            !name.is_empty(),
            "dependency has an empty name"
        );

        match &dep.source {
            DepSource::CratesIo { version } => {
                ensure!(
                    !version.is_empty(),
                    "dependency '{name}' has crates.io source but no version"
                );
                // Basic semver format check: should contain at least one dot.
                ensure!(
                    version.contains('.'),
                    "dependency '{name}' version '{version}' does not look like a semver version (expected X.Y.Z)"
                );
            }
            DepSource::Git { url, .. } => {
                ensure!(
                    !url.is_empty(),
                    "dependency '{name}' has git source but no URL"
                );
                ensure!(
                    url.starts_with("https://") || url.starts_with("http://") || url.starts_with("git://") || url.starts_with("ssh://"),
                    "dependency '{name}' git URL '{url}' does not look like a valid URL"
                );
            }
            DepSource::Path { path } => {
                ensure!(
                    !path.is_empty(),
                    "dependency '{name}' has path source but empty path"
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BuildModel, ConfigOptionDef, ConfigType, ConfigValue, CrateDef, CrateType, DepDef,
        DepSource, ExternalDepDef, GitRef, GroupDef, PipelineStep, ProfileDef, RuleDef,
        RuleHandler, TargetDef,
    };
    use std::collections::BTreeMap;

    /// Build a minimal valid `BuildModel` that passes all validation checks.
    fn minimal_model() -> BuildModel {
        let mut model = BuildModel::default();
        model.project.name = "test".into();
        model.project.version = "0.1.0".into();
        model.targets.insert(
            "x86_64".into(),
            TargetDef {
                name: "x86_64".into(),
                spec: "x86_64-unknown-hadron".into(),
            },
        );
        model.profiles.insert(
            "default".into(),
            ProfileDef {
                name: "default".into(),
                target: Some("x86_64".into()),
                ..Default::default()
            },
        );
        model
            .pipeline
            .steps
            .push(PipelineStep::Barrier("init".into()));
        model
    }

    /// Construct a `ConfigOptionDef` with `Bool` type.
    fn bool_opt(name: &str, default: bool) -> ConfigOptionDef {
        ConfigOptionDef {
            name: name.into(),
            ty: ConfigType::Bool,
            default: ConfigValue::Bool(default),
            help: None,
            depends_on: vec![],
            selects: vec![],
            range: None,
            choices: None,
            menu: None,
            bindings: Vec::new(),
        }
    }

    /// Construct a minimal `CrateDef`.
    fn make_crate(name: &str, target: &str) -> CrateDef {
        CrateDef {
            name: name.into(),
            path: format!("crates/{name}"),
            edition: "2024".into(),
            crate_type: CrateType::Lib,
            target: target.into(),
            deps: BTreeMap::new(),
            dev_deps: BTreeMap::new(),
            features: Vec::new(),
            root: None,
            linker_script: None,
            group: None,
            is_project_crate: true,
            cfg_flags: Vec::new(),
            requires_config: Vec::new(),
        }
    }

    /// Construct a `DepDef` referencing a crate by name.
    fn make_dep(crate_name: &str) -> DepDef {
        DepDef {
            extern_name: crate_name.into(),
            crate_name: crate_name.into(),
            features: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // 1. valid_minimal_model_passes
    // -----------------------------------------------------------------------

    #[test]
    fn valid_minimal_model_passes() {
        let model = minimal_model();
        validate_model(&model).expect("minimal model should pass validation");
    }

    // -----------------------------------------------------------------------
    // 2. missing_project_name_fails
    // -----------------------------------------------------------------------

    #[test]
    fn missing_project_name_fails() {
        let mut model = minimal_model();
        model.project.name = String::new();
        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("project name is required"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 3. missing_project_version_fails
    // -----------------------------------------------------------------------

    #[test]
    fn missing_project_version_fails() {
        let mut model = minimal_model();
        model.project.version = String::new();
        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("project version is required"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 4. missing_default_profile_fails
    // -----------------------------------------------------------------------

    #[test]
    fn missing_default_profile_fails() {
        let mut model = minimal_model();
        model.profiles.remove("default");
        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("'default' profile is required"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 5. profile_inheritance_cycle_detected
    // -----------------------------------------------------------------------

    #[test]
    fn profile_inheritance_cycle_detected() {
        let mut model = minimal_model();

        // A inherits B, B inherits A.
        model.profiles.insert(
            "a".into(),
            ProfileDef {
                name: "a".into(),
                inherits: Some("b".into()),
                ..Default::default()
            },
        );
        model.profiles.insert(
            "b".into(),
            ProfileDef {
                name: "b".into(),
                inherits: Some("a".into()),
                ..Default::default()
            },
        );

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("cycle"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 6. config_type_mismatch_fails
    // -----------------------------------------------------------------------

    #[test]
    fn config_type_mismatch_fails() {
        let mut model = minimal_model();
        model.config_options.insert(
            "bad_opt".into(),
            ConfigOptionDef {
                name: "bad_opt".into(),
                ty: ConfigType::Bool,
                default: ConfigValue::U32(1),
                help: None,
                depends_on: vec![],
                selects: vec![],
                range: None,
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string()
                .contains("does not match declared type"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 7. config_range_min_gt_max_fails
    // -----------------------------------------------------------------------

    #[test]
    fn config_range_min_gt_max_fails() {
        let mut model = minimal_model();
        model.config_options.insert(
            "num_opt".into(),
            ConfigOptionDef {
                name: "num_opt".into(),
                ty: ConfigType::U32,
                default: ConfigValue::U32(50),
                help: None,
                depends_on: vec![],
                selects: vec![],
                range: Some((100, 1)),
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("range min"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 8. config_choice_missing_variants_fails
    // -----------------------------------------------------------------------

    #[test]
    fn config_choice_missing_variants_fails() {
        let mut model = minimal_model();
        model.config_options.insert(
            "choice_opt".into(),
            ConfigOptionDef {
                name: "choice_opt".into(),
                ty: ConfigType::Choice,
                default: ConfigValue::Choice("a".into()),
                help: None,
                depends_on: vec![],
                selects: vec![],
                range: None,
                choices: Some(vec![]),
                menu: None,
                bindings: Vec::new(),
            },
        );

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("no variants"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 9. pipeline_undefined_group_fails
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_undefined_group_fails() {
        let mut model = minimal_model();
        model.pipeline.steps.push(PipelineStep::Stage {
            name: "s1".into(),
            groups: vec!["nonexistent".into()],
        });

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("not defined"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 10. crate_undefined_dep_fails
    // -----------------------------------------------------------------------

    #[test]
    fn crate_undefined_dep_fails() {
        let mut model = minimal_model();

        // Add a crate that depends on "nonexistent", which is neither in
        // model.crates nor model.dependencies.
        let mut krate = make_crate("my_crate", "x86_64");
        krate.deps.insert("nonexistent".into(), make_dep("nonexistent"));
        model.crates.insert("my_crate".into(), krate);

        // The crate must also be in a group for group validation to pass,
        // and the group must be in the pipeline.
        model.groups.insert(
            "grp".into(),
            GroupDef {
                name: "grp".into(),
                target: "x86_64".into(),
                crates: vec!["my_crate".into()],
                ..Default::default()
            },
        );
        model.pipeline.steps.push(PipelineStep::Stage {
            name: "s1".into(),
            groups: vec!["grp".into()],
        });

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("not defined"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 11. dependency_crates_io_no_version_fails
    // -----------------------------------------------------------------------

    #[test]
    fn dependency_crates_io_no_version_fails() {
        let mut model = minimal_model();
        model.dependencies.insert(
            "some_dep".into(),
            ExternalDepDef {
                name: "some_dep".into(),
                source: DepSource::CratesIo {
                    version: String::new(),
                },
                features: Vec::new(),
                default_features: true,
                cfg_flags: Vec::new(),
            },
        );

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("no version"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 12. dependency_git_empty_url_fails
    // -----------------------------------------------------------------------

    #[test]
    fn dependency_git_empty_url_fails() {
        let mut model = minimal_model();
        model.dependencies.insert(
            "git_dep".into(),
            ExternalDepDef {
                name: "git_dep".into(),
                source: DepSource::Git {
                    url: String::new(),
                    reference: GitRef::Default,
                },
                features: Vec::new(),
                default_features: true,
                cfg_flags: Vec::new(),
            },
        );

        let err = validate_model(&model).unwrap_err();
        assert!(
            err.to_string().contains("no URL"),
            "unexpected error: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // 13. valid_model_with_all_features
    // -----------------------------------------------------------------------

    #[test]
    fn valid_model_with_all_features() {
        let mut model = minimal_model();

        // -- Config options --
        model
            .config_options
            .insert("enable_serial".into(), bool_opt("enable_serial", true));
        model.config_options.insert(
            "log_level".into(),
            ConfigOptionDef {
                name: "log_level".into(),
                ty: ConfigType::Choice,
                default: ConfigValue::Choice("info".into()),
                help: Some("Kernel log level".into()),
                depends_on: vec![],
                selects: vec![],
                range: None,
                choices: Some(vec![
                    "error".into(),
                    "warn".into(),
                    "info".into(),
                    "debug".into(),
                ]),
                menu: None,
                bindings: Vec::new(),
            },
        );
        model.config_options.insert(
            "heap_size".into(),
            ConfigOptionDef {
                name: "heap_size".into(),
                ty: ConfigType::U64,
                default: ConfigValue::U64(1024),
                help: None,
                depends_on: vec![],
                selects: vec![],
                range: Some((256, 65536)),
                choices: None,
                menu: None,
                bindings: Vec::new(),
            },
        );

        // -- External dependencies --
        model.dependencies.insert(
            "bitflags".into(),
            ExternalDepDef {
                name: "bitflags".into(),
                source: DepSource::CratesIo {
                    version: "2.6.0".into(),
                },
                features: Vec::new(),
                default_features: true,
                cfg_flags: Vec::new(),
            },
        );

        // -- Crates --
        let mut kernel = make_crate("hadron-kernel", "x86_64");
        kernel.deps.insert("bitflags".into(), make_dep("bitflags"));
        model.crates.insert("hadron-kernel".into(), kernel);

        let mut drivers = make_crate("hadron-drivers", "x86_64");
        drivers
            .deps
            .insert("hadron-kernel".into(), make_dep("hadron-kernel"));
        model.crates.insert("hadron-drivers".into(), drivers);

        // -- Groups --
        model.groups.insert(
            "kernel".into(),
            GroupDef {
                name: "kernel".into(),
                target: "x86_64".into(),
                crates: vec!["hadron-kernel".into(), "hadron-drivers".into()],
                ..Default::default()
            },
        );

        // -- Rules --
        model.rules.insert(
            "link_kernel".into(),
            RuleDef {
                name: "link_kernel".into(),
                inputs: vec!["hadron-kernel".into()],
                outputs: vec!["kernel.elf".into()],
                depends_on: vec![],
                handler: RuleHandler::Builtin("hbtf".into()),
            },
        );

        // -- Pipeline --
        model.pipeline.steps.push(PipelineStep::Stage {
            name: "compile".into(),
            groups: vec!["kernel".into()],
        });
        model
            .pipeline
            .steps
            .push(PipelineStep::Barrier("link_barrier".into()));
        model
            .pipeline
            .steps
            .push(PipelineStep::Rule("link_kernel".into()));

        validate_model(&model).expect("full-featured model should pass validation");
    }
}
