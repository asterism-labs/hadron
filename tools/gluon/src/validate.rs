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

        // All dependency references must resolve.
        for (extern_name, dep) in &krate.deps {
            ensure!(
                model.crates.contains_key(&dep.crate_name),
                "crate '{name}' depends on '{ext}' (crate '{dep_name}'), which is not defined",
                ext = extern_name,
                dep_name = dep.crate_name
            );
        }

        // All dev dependency references must resolve.
        for (extern_name, dep) in &krate.dev_deps {
            ensure!(
                model.crates.contains_key(&dep.crate_name),
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
