//! Barrier + DAG scheduler for the build pipeline.
//!
//! Walks [`PipelineDef`] steps sequentially: stages expand groups into crates
//! and compile them with DAG-ordered parallelism, barriers wait for all prior
//! work, and rules execute artifact generation handlers.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::cache::{CacheManifest, CrateEntry};
use crate::compile::{self, ArtifactMap, CompileMode};
use crate::config::ResolvedConfig;
use crate::crate_graph::{self, ResolvedCrate};
use crate::model::{BuildModel, PipelineStep, RuleHandler};
use crate::sysroot;

/// State accumulated during pipeline execution.
pub struct PipelineState {
    pub config: ResolvedConfig,
    /// Per-target spec paths (target name → absolute spec path).
    pub target_specs: HashMap<String, String>,
    /// Per-target sysroot directories (target name → sysroot dir).
    pub sysroots: HashMap<String, PathBuf>,
    /// Per-target config rlibs (target name → config rlib path).
    pub config_rlibs: HashMap<String, PathBuf>,
    pub artifacts: ArtifactMap,
    pub cache: CacheManifest,
    pub rebuilt: HashSet<String>,
    pub force: bool,
    pub kernel_binary: Option<PathBuf>,
    pub kernel_binary_rebuilt: bool,
    pub total_crates: usize,
    pub recompiled_crates: usize,
}

/// Execute the build pipeline for a full build.
pub fn execute_pipeline(
    model: &BuildModel,
    state: &mut PipelineState,
    mode: CompileMode,
) -> Result<()> {
    let root = state.config.root.clone();
    let sysroot_src = sysroot::sysroot_src_dir()?;

    for step in &model.pipeline.steps {
        match step {
            PipelineStep::Stage { name, groups } => {
                if groups.is_empty() {
                    continue;
                }
                println!("\nCompiling stage '{name}'...");
                execute_stage(model, state, groups, &root, &sysroot_src, mode)?;
            }
            PipelineStep::Barrier(_) => {
                // Barriers are implicit in sequential execution.
            }
            PipelineStep::Rule(rname) => {
                if mode != CompileMode::Build {
                    continue;
                }
                execute_rule(model, state, rname, &root)?;
            }
        }
    }

    Ok(())
}

/// Ensure a sysroot exists for the given target, building it lazily if needed.
fn ensure_sysroot(
    model: &BuildModel,
    state: &mut PipelineState,
    target: &str,
    root: &Path,
) -> Result<()> {
    if state.sysroots.contains_key(target) {
        return Ok(());
    }

    let target_def = model.targets.get(target)
        .ok_or_else(|| anyhow::anyhow!("target '{target}' not found in model"))?;
    let target_spec_path = root.join(&target_def.spec);
    let target_spec = target_spec_path
        .to_str()
        .expect("target spec path is valid UTF-8")
        .to_string();

    // Store the spec path for this target.
    state.target_specs.insert(target.to_string(), target_spec);

    // Build or reuse sysroot.
    println!("  Building sysroot for {target}...");
    let sysroot_dir = if !state.force
        && state.cache
            .is_sysroot_fresh(target, state.config.profile.opt_level)
            .is_fresh()
    {
        println!("  Sysroot unchanged, skipping.");
        sysroot::sysroot_output_paths(root, target).sysroot_dir
    } else {
        let sysroot_output = sysroot::build_sysroot(
            root,
            &target_spec_path,
            target,
            state.config.profile.opt_level,
        )?;
        state.cache.record_sysroot(
            target,
            state.config.profile.opt_level,
            sysroot_output.core_rlib,
            sysroot_output.compiler_builtins_rlib,
            sysroot_output.alloc_rlib,
        );
        println!("  Sysroot ready.");
        sysroot_output.sysroot_dir
    };

    state.sysroots.insert(target.to_string(), sysroot_dir);
    Ok(())
}

/// Ensure a config crate exists for the given target, building it lazily if needed.
fn ensure_config_crate(
    state: &mut PipelineState,
    target: &str,
) -> Result<()> {
    if state.config_rlibs.contains_key(target) {
        return Ok(());
    }

    let target_spec = state.target_specs.get(target)
        .ok_or_else(|| anyhow::anyhow!("target spec for '{target}' not resolved"))?
        .clone();
    let sysroot_dir = state.sysroots.get(target)
        .ok_or_else(|| anyhow::anyhow!("sysroot for '{target}' not built"))?
        .clone();

    println!("  Generating hadron_config for {target}...");
    let config_dep_info = compile::config_crate_dep_info_path(&state.config);

    let config_flags_hash = {
        let mut parts: Vec<&std::ffi::OsStr> = vec!["hadron_config".as_ref()];
        let opt_str = format!("{}", state.config.profile.opt_level);
        parts.push(opt_str.as_ref());
        parts.push(target_spec.as_ref());
        compile::hash_args(&parts)
    };

    let config_rlib_path = state.config.root
        .join("build/kernel")
        .join(target)
        .join("debug/libhadron_config.rlib");

    let config_needs_rebuild = state.force || {
        let key = format!("hadron_config_{target}");
        match state.cache.entries.get_mut(&key) {
            Some(entry) => !entry.is_fresh(&config_flags_hash, &state.rebuilt, &[]).is_fresh(),
            None => true,
        }
    };

    let config_rlib = if config_needs_rebuild {
        let rlib = compile::build_config_crate(&state.config, &target_spec, &sysroot_dir)?;
        let key = format!("hadron_config_{target}");
        if let Ok(entry) = CrateEntry::from_compilation(config_flags_hash, &rlib, &config_dep_info) {
            state.cache.entries.insert(key, entry);
        }
        state.rebuilt.insert(format!("hadron_config_{target}"));
        rlib
    } else {
        println!("  Skipping hadron_config (unchanged)");
        config_rlib_path
    };
    state.artifacts.insert(&format!("hadron_config_{target}"), config_rlib.clone());
    state.config_rlibs.insert(target.to_string(), config_rlib);

    Ok(())
}

/// Execute a single pipeline stage: expand groups, toposort, compile.
fn execute_stage(
    model: &BuildModel,
    state: &mut PipelineState,
    group_names: &[String],
    root: &Path,
    sysroot_src: &Path,
    mode: CompileMode,
) -> Result<()> {
    // For each group, ensure sysroot and config are ready, then collect crates.
    let mut all_crates = Vec::new();
    let mut group_config_map: HashMap<String, bool> = HashMap::new();

    for gname in group_names {
        let group = model.groups.get(gname)
            .ok_or_else(|| anyhow::anyhow!("group '{gname}' not found"))?;

        // Ensure sysroot for non-host targets.
        if group.target != "host" {
            ensure_sysroot(model, state, &group.target, root)?;
        }

        // Ensure config crate for config-enabled groups.
        if group.config && group.target != "host" {
            ensure_config_crate(state, &group.target)?;
        }

        // Track which groups have config enabled.
        group_config_map.insert(gname.clone(), group.config);

        // Resolve and collect crates.
        let resolved = crate_graph::resolve_group_from_model(
            model,
            gname,
            root,
            sysroot_src,
        )?;

        for krate in resolved {
            // Skip sysroot crates (paths starting with {sysroot}/) — they're
            // compiled by sysroot::build_sysroot(), not the regular pipeline.
            let model_def = model.crates.get(&krate.name);
            if let Some(def) = model_def {
                if def.path.starts_with("{sysroot}/") {
                    continue;
                }
            }

            // Avoid duplicates (a crate might be in multiple groups).
            if !all_crates.iter().any(|k: &(ResolvedCrate, bool)| k.0.name == krate.name) {
                all_crates.push((krate, group.config));
            }
        }
    }

    if all_crates.is_empty() {
        return Ok(());
    }

    // Quick stage check.
    let stage_names: Vec<String> = all_crates.iter().map(|(k, _)| k.name.clone()).collect();
    let total = all_crates.len();
    state.total_crates += total;

    if !state.force && state.cache.is_stage_fresh(&stage_names, &state.rebuilt) {
        println!("  All {total} crates unchanged, skipping.");
        for (krate, _) in &all_crates {
            let artifact_path = compile::crate_artifact_path(krate, root, None, mode);
            if krate.name == state.config.profile.boot_binary {
                state.kernel_binary = Some(artifact_path.clone());
            }
            state.artifacts.insert(&krate.name, artifact_path);
        }
        return Ok(());
    }

    // Compile each crate in topological order.
    for (krate, has_config) in &all_crates {
        let is_host = krate.target == "host";
        let artifact_path = compile::crate_artifact_path(krate, root, None, mode);
        let dep_info_path = compile::crate_dep_info_path(krate, root, None);
        let dep_names: Vec<String> = krate.deps.iter().map(|d| d.crate_name.clone()).collect();

        let mode_tag = match mode {
            CompileMode::Build if is_host => "host",
            CompileMode::Build => "kernel",
            CompileMode::Check => "check",
            CompileMode::Clippy => "clippy",
        };

        let flags_hash = if is_host {
            compile::hash_args(&[
                "host".as_ref(),
                krate.name.as_ref(),
                krate.edition.as_ref(),
                krate.crate_type.as_ref(),
            ])
        } else {
            let target_spec = state.target_specs.get(&krate.target).map(|s| s.as_str()).unwrap_or("");
            compile::hash_args(&[
                mode_tag.as_ref(),
                krate.name.as_ref(),
                krate.edition.as_ref(),
                krate.crate_type.as_ref(),
                format!("{}", state.config.profile.opt_level).as_ref(),
                target_spec.as_ref(),
            ])
        };

        // Check cache freshness.
        if !state.force {
            if let Some(entry) = state.cache.entries.get_mut(&krate.name) {
                if entry.is_fresh(&flags_hash, &state.rebuilt, &dep_names).is_fresh() {
                    println!("  Skipping {} (unchanged)", krate.name);
                    if krate.name == state.config.profile.boot_binary {
                        state.kernel_binary = Some(artifact_path.clone());
                    }
                    state.artifacts.insert(&krate.name, artifact_path);
                    continue;
                }
            }
        }

        let verb = match mode {
            CompileMode::Build => "Compiling",
            CompileMode::Check => "Checking",
            CompileMode::Clippy => "Checking",
        };
        let ctx_tag = if is_host { " (host)" } else { "" };
        println!("  {verb} {}{}...", krate.name, ctx_tag);

        let target_spec = state.target_specs.get(&krate.target).map(|s| s.as_str());
        let sysroot_dir = state.sysroots.get(&krate.target).map(|p| p.as_path());
        let config_rlib = if *has_config {
            state.config_rlibs.get(&krate.target).map(|p| p.as_path())
        } else {
            None
        };

        let artifact = compile::compile_crate(
            krate,
            &state.config,
            target_spec,
            sysroot_dir,
            &state.artifacts,
            config_rlib,
            None,
            mode,
        )?;

        if let Ok(entry) = CrateEntry::from_compilation(flags_hash, &artifact, &dep_info_path) {
            state.cache.entries.insert(krate.name.clone(), entry);
        }
        if krate.name == state.config.profile.boot_binary {
            state.kernel_binary = Some(artifact.clone());
            state.kernel_binary_rebuilt = true;
        }
        state.rebuilt.insert(krate.name.clone());
        state.artifacts.insert(&krate.name, artifact);
        state.recompiled_crates += 1;
    }

    Ok(())
}

/// Execute a named rule.
fn execute_rule(
    model: &BuildModel,
    state: &mut PipelineState,
    rule_name: &str,
    root: &Path,
) -> Result<()> {
    let rule = model.rules.get(rule_name)
        .ok_or_else(|| anyhow::anyhow!("rule '{rule_name}' not found"))?;

    match &rule.handler {
        RuleHandler::Builtin(handler_name) => {
            match handler_name.as_str() {
                "hbtf" => {
                    if let Some(ref kernel_bin) = state.kernel_binary {
                        if state.kernel_binary_rebuilt || state.force {
                            let hbtf_path = root.join("build/backtrace.hbtf");
                            println!("\nGenerating HBTF...");
                            crate::artifact::hbtf::generate_hbtf(
                                kernel_bin,
                                &hbtf_path,
                                state.config.profile.debug_info,
                            )?;
                            let target_hbtf = root.join("target/backtrace.hbtf");
                            std::fs::create_dir_all(target_hbtf.parent().unwrap())?;
                            std::fs::copy(&hbtf_path, &target_hbtf)?;
                        } else {
                            println!("\nHBTF unchanged, skipping.");
                        }
                    }
                }
                "initrd" => {
                    let initrd_path = root.join("build/initrd.cpio");
                    let target_initrd = root.join("target/initrd.cpio");

                    // Collect already-compiled binary artifacts from the rule's inputs.
                    let mut bin_artifacts = Vec::new();
                    for input in &rule.inputs {
                        if let Some(path) = state.artifacts.get(input) {
                            // Only include binary crate artifacts.
                            if let Some(def) = model.crates.get(input) {
                                if def.crate_type == crate::model::CrateType::Bin {
                                    bin_artifacts.push((input.clone(), path.to_path_buf()));
                                }
                            }
                        }
                    }

                    // Track source roots for freshness.
                    let sysroot_src = sysroot::sysroot_src_dir()?;
                    let user_source_roots: Vec<PathBuf> = rule.inputs.iter()
                        .filter_map(|name| model.crates.get(name))
                        .filter_map(|def| {
                            let resolved_path = root.join(&def.path);
                            Some(def.root_file(&resolved_path))
                        })
                        .collect();
                    // Silence unused variable warning for sysroot_src.
                    let _ = sysroot_src;

                    let initrd_fresh = !state.force
                        && state.cache.is_initrd_fresh(&initrd_path, &user_source_roots)
                        && target_initrd.exists();

                    if initrd_fresh {
                        println!("\nInitrd unchanged, skipping.");
                    } else {
                        println!("\nBuilding initrd...");
                        let built = crate::artifact::initrd::build_initrd(
                            &state.config,
                            &bin_artifacts,
                        )?;
                        state.cache.record_initrd(&built, &user_source_roots);
                        state.cache.save(&state.config.root)?;

                        std::fs::create_dir_all(target_initrd.parent().unwrap())?;
                        std::fs::copy(&built, &target_initrd)?;
                    }
                }
                "config_crate" => {
                    // Config crate is built lazily by ensure_config_crate().
                }
                other => {
                    bail!("unknown built-in rule handler: '{other}'");
                }
            }
        }
        RuleHandler::Script(fn_name) => {
            bail!("script rule handlers are not yet implemented (rule: '{rule_name}', fn: '{fn_name}')");
        }
    }

    Ok(())
}
