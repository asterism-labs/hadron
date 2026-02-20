//! Barrier + DAG scheduler for the build pipeline.
//!
//! Walks [`PipelineDef`] steps sequentially: stages expand groups into crates
//! and compile them with DAG-ordered parallelism, barriers wait for all prior
//! work, and rules execute artifact generation handlers.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock, mpsc};

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
                let _t = crate::verbose::Timer::start("stage");
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

// ---------------------------------------------------------------------------
// Parallel DAG compilation
// ---------------------------------------------------------------------------

/// A compilation job dispatched to a worker thread.
struct CompileJob {
    /// Index into the stage's `all_crates` vector.
    krate_idx: usize,
    /// Pre-computed flags hash for cache recording.
    flags_hash: String,
    /// Whether this crate's group has config enabled.
    has_config: bool,
    mode: CompileMode,
}

/// Result sent back from a worker thread.
enum CompileOutcome {
    /// Compilation succeeded.
    Compiled {
        krate_idx: usize,
        artifact: PathBuf,
        flags_hash: String,
    },
    /// Compilation failed.
    Error {
        krate_idx: usize,
        error: anyhow::Error,
    },
}

/// Execute a single pipeline stage: expand groups, toposort, compile with parallelism.
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

    // Crate gating: remove crates whose requires_config options are disabled.
    all_crates.retain(|(krate, _)| {
        let crate_def = model.crates.get(&krate.name);
        if let Some(def) = crate_def {
            for req in &def.requires_config {
                match state.config.options.get(req) {
                    Some(crate::config::ResolvedValue::Bool(true)) => {}
                    _ => {
                        println!("  Skipping {} (requires config '{req}' which is disabled)", krate.name);
                        return false;
                    }
                }
            }
        }
        true
    });

    if all_crates.is_empty() {
        return Ok(());
    }

    // Cross-group topological sort: re-order all_crates so that dependencies
    // from other groups in the same stage are compiled first.
    toposort_stage_crates(&mut all_crates);

    let total = all_crates.len();
    state.total_crates += total;

    // Build in-degree map and forward adjacency for DAG scheduling.
    // Only count dependencies that are within this stage.
    let name_to_idx: HashMap<&str, usize> = all_crates.iter()
        .enumerate()
        .map(|(i, (k, _))| (k.name.as_str(), i))
        .collect();

    let mut in_degree: Vec<usize> = vec![0; total];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); total];

    for (idx, (krate, _)) in all_crates.iter().enumerate() {
        for dep in &krate.deps {
            if let Some(&dep_idx) = name_to_idx.get(dep.crate_name.as_str()) {
                in_degree[idx] += 1;
                dependents[dep_idx].push(idx);
            }
        }
    }

    // Seed the ready queue with zero-in-degree crates.
    let mut ready_queue: Vec<usize> = (0..total)
        .filter(|&i| in_degree[i] == 0)
        .collect();

    let num_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8);

    crate::verbose::vprintln!("  parallel compilation: {} workers, {} crates", num_workers, total);

    // Move artifacts into shared RwLock for concurrent access.
    let shared_artifacts = RwLock::new(std::mem::take(&mut state.artifacts));

    // Create channels.
    let (job_tx, job_rx) = mpsc::channel::<CompileJob>();
    let (result_tx, result_rx) = mpsc::channel::<CompileOutcome>();
    let job_rx = Mutex::new(job_rx);

    // References for workers (captured by thread::scope closures).
    let all_crates_ref = &all_crates;
    let config_ref = &state.config;
    let target_specs_ref = &state.target_specs;
    let sysroots_ref = &state.sysroots;
    let config_rlibs_ref = &state.config_rlibs;
    let shared_artifacts_ref = &shared_artifacts;
    let job_rx_ref = &job_rx;

    let stage_result: Result<()> = std::thread::scope(|s| {
        // Spawn worker threads.
        for _ in 0..num_workers {
            let tx = result_tx.clone();
            s.spawn(move || {
                loop {
                    let job = match job_rx_ref.lock().unwrap().recv() {
                        Ok(j) => j,
                        Err(_) => break, // channel closed, exit
                    };

                    let (krate, _) = &all_crates_ref[job.krate_idx];
                    let target_spec = target_specs_ref.get(&krate.target)
                        .map(|s| s.as_str());
                    let sysroot_dir = sysroots_ref.get(&krate.target)
                        .map(|p| p.as_path());
                    let config_rlib = if job.has_config {
                        config_rlibs_ref.get(&krate.target).map(|p| p.as_path())
                    } else {
                        None
                    };

                    let arts = shared_artifacts_ref.read().unwrap();
                    let result = compile::compile_crate(
                        krate,
                        config_ref,
                        target_spec,
                        sysroot_dir,
                        &arts,
                        config_rlib,
                        None,
                        job.mode,
                    );
                    drop(arts);

                    let outcome = match result {
                        Ok(artifact) => CompileOutcome::Compiled {
                            krate_idx: job.krate_idx,
                            artifact,
                            flags_hash: job.flags_hash,
                        },
                        Err(error) => CompileOutcome::Error {
                            krate_idx: job.krate_idx,
                            error,
                        },
                    };
                    if tx.send(outcome).is_err() {
                        break;
                    }
                }
            });
        }

        // Drop the original result_tx so the channel closes when all workers finish.
        drop(result_tx);

        // --- Main thread: dispatch jobs and process results ---
        let boot_binary_name = &state.config.profile.boot_binary;
        let mut compiled_count = 0usize;
        let mut in_flight = 0usize;
        let mut first_error: Option<anyhow::Error> = None;

        while compiled_count < total {
            // Dispatch all ready crates.
            let batch: Vec<usize> = ready_queue.drain(..).collect();
            for idx in batch {
                if first_error.is_some() {
                    break;
                }

                let (krate, has_config) = &all_crates[idx];
                let is_host = krate.target == "host";
                let artifact_path = compile::crate_artifact_path(krate, root, None, mode);
                let dep_names: Vec<String> = krate.deps.iter()
                    .map(|d| d.crate_name.clone())
                    .collect();

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
                    let target_spec = state.target_specs
                        .get(&krate.target)
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    compile::hash_args(&[
                        mode_tag.as_ref(),
                        krate.name.as_ref(),
                        krate.edition.as_ref(),
                        krate.crate_type.as_ref(),
                        format!("{}", state.config.profile.opt_level).as_ref(),
                        target_spec.as_ref(),
                    ])
                };

                // Check cache freshness (main thread only — mutates cache).
                if !state.force {
                    if let Some(entry) = state.cache.entries.get_mut(&krate.name) {
                        let freshness = entry.is_fresh(
                            &flags_hash,
                            &state.rebuilt,
                            &dep_names,
                        );
                        if freshness.is_fresh() {
                            println!("  Skipping {} (unchanged)", krate.name);
                            if krate.name == *boot_binary_name {
                                state.kernel_binary = Some(artifact_path.clone());
                            }
                            shared_artifacts.write().unwrap()
                                .insert(&krate.name, artifact_path);
                            compiled_count += 1;

                            // Decrement dependents' in-degree.
                            for &dep_idx in &dependents[idx] {
                                in_degree[dep_idx] -= 1;
                                if in_degree[dep_idx] == 0 {
                                    ready_queue.push(dep_idx);
                                }
                            }
                            continue;
                        }
                        if let crate::cache::FreshResult::Stale(ref reason) = freshness {
                            crate::verbose::vprintln!(
                                "  stale: {} — {}",
                                krate.name,
                                reason
                            );
                        }
                    }
                }

                // Not cached — dispatch to worker.
                let verb = match mode {
                    CompileMode::Build => "Compiling",
                    CompileMode::Check => "Checking",
                    CompileMode::Clippy => "Checking",
                };
                let ctx_tag = if is_host { " (host)" } else { "" };
                println!("  {verb} {}{}...", krate.name, ctx_tag);

                let _ = job_tx.send(CompileJob {
                    krate_idx: idx,
                    flags_hash,
                    has_config: *has_config,
                    mode,
                });
                in_flight += 1;
            }

            // If nothing in flight and nothing ready, check if we're done or stuck.
            if in_flight == 0 {
                if compiled_count >= total {
                    break;
                }
                if ready_queue.is_empty() {
                    bail!("dependency cycle detected: {} of {} crates cannot be scheduled",
                        total - compiled_count, total);
                }
                // There are newly-ready crates from cache skips; loop back to dispatch them.
                continue;
            }

            if first_error.is_some() {
                // Drain remaining in-flight results before returning.
                while in_flight > 0 {
                    if result_rx.recv().is_ok() {
                        in_flight -= 1;
                    } else {
                        break;
                    }
                }
                // Close job channel to shut down workers.
                drop(job_tx);
                return Err(first_error.unwrap());
            }

            // Wait for one result.
            match result_rx.recv() {
                Ok(CompileOutcome::Compiled { krate_idx, artifact, flags_hash }) => {
                    in_flight -= 1;
                    compiled_count += 1;

                    let (krate, _) = &all_crates[krate_idx];
                    let dep_info_path = compile::crate_dep_info_path(krate, root, None);

                    // Update cache.
                    if let Ok(entry) = CrateEntry::from_compilation(
                        flags_hash,
                        &artifact,
                        &dep_info_path,
                    ) {
                        state.cache.entries.insert(krate.name.clone(), entry);
                    }

                    // Track kernel binary.
                    if krate.name == *boot_binary_name {
                        state.kernel_binary = Some(artifact.clone());
                        state.kernel_binary_rebuilt = true;
                    }

                    state.rebuilt.insert(krate.name.clone());
                    shared_artifacts.write().unwrap()
                        .insert(&krate.name, artifact);
                    state.recompiled_crates += 1;

                    // Decrement dependents' in-degree.
                    for &dep_idx in &dependents[krate_idx] {
                        in_degree[dep_idx] -= 1;
                        if in_degree[dep_idx] == 0 {
                            ready_queue.push(dep_idx);
                        }
                    }
                }
                Ok(CompileOutcome::Error { krate_idx, error }) => {
                    in_flight -= 1;
                    let (krate, _) = &all_crates[krate_idx];
                    first_error = Some(anyhow::anyhow!(
                        "failed to compile '{}': {error}",
                        krate.name,
                    ));
                    // Close job channel to stop new dispatches.
                    drop(job_tx);
                    // Drain remaining.
                    while in_flight > 0 {
                        if result_rx.recv().is_ok() {
                            in_flight -= 1;
                        } else {
                            break;
                        }
                    }
                    return Err(first_error.unwrap());
                }
                Err(_) => {
                    // All workers dropped — shouldn't happen if in_flight > 0.
                    bail!("worker threads terminated unexpectedly");
                }
            }
        }

        // Close job channel to shut down workers.
        drop(job_tx);

        Ok(())
    });

    // Move artifacts back from RwLock into state.
    state.artifacts = shared_artifacts.into_inner().unwrap();

    stage_result
}

/// Re-order crates across all groups in a stage using topological sort.
///
/// Within a single pipeline stage, multiple groups may contribute crates that
/// depend on each other (e.g. vendored `hadris-common` depends on project
/// `noalloc` from `kernel-libs`). The per-group toposort in `crate_graph.rs`
/// only orders within a group. This function does a unified sort across all
/// crates collected for the stage.
fn toposort_stage_crates(crates: &mut Vec<(ResolvedCrate, bool)>) {
    use std::collections::VecDeque;

    let name_set: HashSet<&str> = crates.iter().map(|(k, _)| k.name.as_str()).collect();

    // Build in-degree and forward adjacency (dep → dependents).
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for (krate, _) in crates.iter() {
        in_degree.insert(&krate.name, 0);
    }

    for (krate, _) in crates.iter() {
        for dep in &krate.deps {
            if name_set.contains(dep.crate_name.as_str()) {
                *in_degree.entry(krate.name.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.crate_name.as_str())
                    .or_default()
                    .push(&krate.name);
            }
        }
    }

    // Kahn's algorithm.
    let mut queue: VecDeque<&str> = VecDeque::new();
    for (&name, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(name);
        }
    }

    let mut sorted_names: Vec<String> = Vec::with_capacity(crates.len());
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

    // If we couldn't sort all crates (cycle), leave them as-is.
    if sorted_names.len() != crates.len() {
        return;
    }

    // Build index map and reorder in-place.
    let order: HashMap<&str, usize> = sorted_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    crates.sort_by_key(|(k, _)| order.get(k.name.as_str()).copied().unwrap_or(usize::MAX));
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
                "hkif" => {
                    if let Some(ref kernel_bin) = state.kernel_binary {
                        if state.kernel_binary_rebuilt || state.force {
                            println!("\nGenerating HKIF (two-pass link)...");
                            let hkif_bin = root.join("build/hkif.bin");
                            let hkif_asm = root.join("build/hkif.S");
                            let hkif_obj = crate::artifact::hkif::hkif_object_path(root);

                            // Step 1: Generate HKIF blob from pass-1 kernel ELF.
                            crate::artifact::hkif::generate_hkif(
                                kernel_bin,
                                &hkif_bin,
                                state.config.profile.debug_info,
                            )?;

                            // Step 2: Generate assembly stub.
                            crate::artifact::hkif::generate_hkif_asm(&hkif_bin, &hkif_asm)?;

                            // Step 3: Assemble to object file.
                            crate::artifact::hkif::assemble_hkif(&hkif_asm, &hkif_obj)?;

                            // Step 4: Re-link kernel with HKIF object (pass 2).
                            let boot_name = &state.config.profile.boot_binary;
                            let sysroot_src = sysroot::sysroot_src_dir()?;
                            let group_crates = crate_graph::resolve_group_from_model(
                                model, "kernel-main", root, &sysroot_src,
                            )?;
                            let boot_crate = group_crates.iter()
                                .find(|k| k.name == *boot_name)
                                .ok_or_else(|| anyhow::anyhow!(
                                    "boot binary '{}' not found in kernel-main group", boot_name
                                ))?;

                            let target_spec = state.target_specs.get(&boot_crate.target)
                                .ok_or_else(|| anyhow::anyhow!(
                                    "target spec for '{}' not found", boot_crate.target
                                ))?;
                            let sysroot_dir = state.sysroots.get(&boot_crate.target)
                                .ok_or_else(|| anyhow::anyhow!(
                                    "sysroot for '{}' not found", boot_crate.target
                                ))?;
                            let config_rlib = state.config_rlibs.get(&boot_crate.target)
                                .map(|p| p.as_path());

                            println!("  Re-linking {} with HKIF object...", boot_name);
                            let new_binary = compile::relink_with_objects(
                                boot_crate,
                                &state.config,
                                target_spec,
                                sysroot_dir,
                                &state.artifacts,
                                config_rlib,
                                &[hkif_obj],
                            )?;

                            state.kernel_binary = Some(new_binary.clone());
                            state.artifacts.insert(boot_name, new_binary);
                        } else {
                            println!("\nHKIF unchanged, skipping.");
                        }
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
