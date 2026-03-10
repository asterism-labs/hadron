//! Unified global DAG scheduler for the build pipeline.
//!
//! Builds a single DAG containing all compilation units across all pipeline
//! stages, with edges based on actual dependencies rather than stage barriers.
//! This allows host proc-macros to compile in parallel with sysroot builds,
//! and userspace crates to overlap with late kernel compilation.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use crate::cache::{CacheManifest, CrateEntry};
use crate::compile::{self, ArtifactMap, CompileMode};
use crate::config::ResolvedConfig;
use crate::crate_graph;
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
    /// Maximum number of parallel workers (0 = auto-detect from CPU count).
    pub max_workers: usize,
    /// Total wall-clock time for the DAG pipeline execution.
    pub pipeline_elapsed: Option<Duration>,
}

// ---------------------------------------------------------------------------
// Global DAG node types
// ---------------------------------------------------------------------------

/// A node in the unified build DAG.
enum DagNode {
    /// Build a sysroot for a specific target.
    Sysroot { target: String },
    /// Build the config crate for a specific target.
    ConfigCrate { target: String },
    /// Compile a regular crate. `krate_idx` indexes into the shared `all_crates` vec.
    Crate { krate_idx: usize, has_config: bool },
    /// Execute a named rule (e.g. hbtf, initrd, hkif).
    Rule { name: String },
}

/// Result sent back from a worker thread.
enum CompileOutcome {
    /// Crate compilation succeeded.
    Compiled {
        node_idx: usize,
        artifact: PathBuf,
        flags_hash: String,
        duration: Duration,
    },
    /// Crate compilation failed.
    Error {
        node_idx: usize,
        error: anyhow::Error,
    },
}

/// A compilation job dispatched to a worker thread.
struct CompileJob {
    node_idx: usize,
    krate_idx: usize,
    flags_hash: String,
    has_config: bool,
    mode: CompileMode,
}

/// Execute the build pipeline using a unified global DAG.
pub fn execute_pipeline(
    model: &BuildModel,
    state: &mut PipelineState,
    mode: CompileMode,
) -> Result<()> {
    let root = state.config.root.clone();
    let sysroot_src = sysroot::sysroot_src_dir()?;

    // Pre-resolve all target specs eagerly so workers can reference them.
    for (_name, target_def) in &model.targets {
        let target_spec = if target_def.builtin {
            // Builtin target: pass the triple directly to --target.
            target_def.spec.clone()
        } else {
            // Custom target: resolve relative spec path to absolute.
            let target_spec_path = root.join(&target_def.spec);
            target_spec_path
                .to_str()
                .expect("target spec path is valid UTF-8")
                .to_string()
        };
        state
            .target_specs
            .insert(target_def.name.clone(), target_spec);
    }

    // Resolve all crates from all groups across all pipeline stages.
    let all_crates =
        crate_graph::resolve_all_groups(model, &root, &sysroot_src, &state.config.options)?;

    if all_crates.is_empty() {
        return Ok(());
    }

    // Collect the set of non-host targets and config-enabled targets.
    let mut sysroot_targets: HashSet<String> = HashSet::new();
    let mut config_targets: HashSet<String> = HashSet::new();

    for (krate, has_config) in &all_crates {
        if krate.target != "host" {
            sysroot_targets.insert(krate.target.clone());
            if *has_config {
                config_targets.insert(krate.target.clone());
            }
        }
    }

    // Collect pipeline rules (in order).
    let rules: Vec<String> = model
        .pipeline
        .steps
        .iter()
        .filter_map(|step| {
            if let PipelineStep::Rule(name) = step {
                if mode == CompileMode::Build {
                    Some(name.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Build DAG nodes.
    let mut nodes: Vec<DagNode> = Vec::new();
    let mut node_name_to_idx: HashMap<String, usize> = HashMap::new();

    // Add Sysroot nodes (one per non-host target).
    for target in &sysroot_targets {
        let idx = nodes.len();
        node_name_to_idx.insert(format!("__sysroot_{target}"), idx);
        nodes.push(DagNode::Sysroot {
            target: target.clone(),
        });
    }

    // Add ConfigCrate nodes (one per config-enabled target).
    for target in &config_targets {
        let idx = nodes.len();
        node_name_to_idx.insert(format!("__config_{target}"), idx);
        nodes.push(DagNode::ConfigCrate {
            target: target.clone(),
        });
    }

    // Add Crate nodes.
    for (i, (krate, has_config)) in all_crates.iter().enumerate() {
        let idx = nodes.len();
        node_name_to_idx.insert(krate.name.clone(), idx);
        nodes.push(DagNode::Crate {
            krate_idx: i,
            has_config: *has_config,
        });
    }

    // Add Rule nodes.
    for rule_name in &rules {
        let idx = nodes.len();
        node_name_to_idx.insert(format!("__rule_{rule_name}"), idx);
        nodes.push(DagNode::Rule {
            name: rule_name.clone(),
        });
    }

    let total = nodes.len();

    // Build adjacency (dependents) and in-degree with edge deduplication.
    // Without dedup, a crate with both barrier and Rust dep edges from the same
    // predecessor gets inflated in-degree, causing deadlock.
    let mut in_degree: Vec<usize> = vec![0; total];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut edge_set: HashSet<(usize, usize)> = HashSet::new();

    let add_edge = |from: usize,
                    to: usize,
                    edge_set: &mut HashSet<(usize, usize)>,
                    in_degree: &mut Vec<usize>,
                    dependents: &mut Vec<Vec<usize>>| {
        if edge_set.insert((from, to)) {
            in_degree[to] += 1;
            dependents[from].push(to);
        }
    };

    for (krate, has_config) in &all_crates {
        let crate_node_idx = *node_name_to_idx.get(&krate.name).unwrap();

        // Non-host crates depend on their target's Sysroot node.
        if krate.target != "host" {
            if let Some(&sysroot_idx) = node_name_to_idx.get(&format!("__sysroot_{}", krate.target))
            {
                add_edge(
                    sysroot_idx,
                    crate_node_idx,
                    &mut edge_set,
                    &mut in_degree,
                    &mut dependents,
                );
            }

            // Config-enabled crates depend on their target's ConfigCrate node.
            if *has_config {
                if let Some(&config_idx) =
                    node_name_to_idx.get(&format!("__config_{}", krate.target))
                {
                    add_edge(
                        config_idx,
                        crate_node_idx,
                        &mut edge_set,
                        &mut in_degree,
                        &mut dependents,
                    );
                }
            }
        }

        // Crate-to-crate edges from resolved dependencies.
        for dep in &krate.deps {
            if let Some(&dep_node_idx) = node_name_to_idx.get(&dep.crate_name) {
                add_edge(
                    dep_node_idx,
                    crate_node_idx,
                    &mut edge_set,
                    &mut in_degree,
                    &mut dependents,
                );
            }
        }
    }

    // ConfigCrate depends on its Sysroot.
    for target in &config_targets {
        if let (Some(&config_idx), Some(&sysroot_idx)) = (
            node_name_to_idx.get(&format!("__config_{target}")),
            node_name_to_idx.get(&format!("__sysroot_{target}")),
        ) {
            add_edge(
                sysroot_idx,
                config_idx,
                &mut edge_set,
                &mut in_degree,
                &mut dependents,
            );
        }
    }

    // Rule nodes depend on their declared inputs (crates) and other rules.
    for rule_name in &rules {
        let rule_node_idx = *node_name_to_idx
            .get(&format!("__rule_{rule_name}"))
            .unwrap();
        if let Some(rule) = model.rules.get(rule_name) {
            for input in &rule.inputs {
                if let Some(&input_idx) = node_name_to_idx.get(input) {
                    add_edge(
                        input_idx,
                        rule_node_idx,
                        &mut edge_set,
                        &mut in_degree,
                        &mut dependents,
                    );
                }
            }
            for dep_rule in &rule.depends_on {
                if let Some(&dep_idx) = node_name_to_idx.get(&format!("__rule_{dep_rule}")) {
                    add_edge(
                        dep_idx,
                        rule_node_idx,
                        &mut edge_set,
                        &mut in_degree,
                        &mut dependents,
                    );
                }
            }
        }
    }

    // Barrier edges: partition pipeline stages by barriers, then add edges
    // from all crates in the preceding segment to all crates in the following segment.
    {
        let mut segments: Vec<Vec<usize>> = Vec::new();
        let mut current_segment: Vec<usize> = Vec::new();

        for step in &model.pipeline.steps {
            match step {
                PipelineStep::Stage { groups, .. } => {
                    for gname in groups {
                        if let Some(group) = model.groups.get(gname) {
                            for crate_name in &group.crates {
                                if let Some(&idx) = node_name_to_idx.get(crate_name) {
                                    current_segment.push(idx);
                                }
                            }
                        }
                    }
                }
                PipelineStep::Rule(rule_name) => {
                    if let Some(&idx) = node_name_to_idx.get(&format!("__rule_{rule_name}")) {
                        current_segment.push(idx);
                    }
                }
                PipelineStep::Barrier(_) => {
                    if !current_segment.is_empty() {
                        segments.push(std::mem::take(&mut current_segment));
                    }
                }
            }
        }
        // Push the final segment (after the last barrier, or the only segment if no barriers).
        if !current_segment.is_empty() {
            segments.push(current_segment);
        }

        // Add edges from each segment to the next (adjacent segments only).
        for i in 0..segments.len().saturating_sub(1) {
            for &pre in &segments[i] {
                for &post in &segments[i + 1] {
                    add_edge(pre, post, &mut edge_set, &mut in_degree, &mut dependents);
                }
            }
        }
    }

    // Artifact dependency edges: ordering-only edges without --extern flags.
    for (krate, _) in &all_crates {
        if let Some(def) = model.crates.get(&krate.name) {
            let crate_idx = *node_name_to_idx.get(&krate.name).unwrap();
            for art_dep in &def.artifact_deps {
                if let Some(&dep_idx) = node_name_to_idx
                    .get(art_dep)
                    .or_else(|| node_name_to_idx.get(&format!("__rule_{art_dep}")))
                {
                    add_edge(
                        dep_idx,
                        crate_idx,
                        &mut edge_set,
                        &mut in_degree,
                        &mut dependents,
                    );
                }
            }
        }
    }

    // Seed the ready queue.
    let mut ready_queue: Vec<usize> = (0..total).filter(|&i| in_degree[i] == 0).collect();

    let num_workers = match state.max_workers {
        0 => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
        n => n,
    };

    let crate_count = all_crates.len();
    state.total_crates += crate_count;
    crate::verbose::vprintln!(
        "  unified DAG: {} nodes ({} crates, {} sysroots, {} configs, {} rules), {} workers",
        total,
        crate_count,
        sysroot_targets.len(),
        config_targets.len(),
        rules.len(),
        num_workers,
    );

    // Move shared state into RwLocks for concurrent access.
    // Workers acquire read locks; main thread acquires brief write locks for inserts.
    let shared_artifacts = RwLock::new(std::mem::take(&mut state.artifacts));
    let shared_target_specs = RwLock::new(std::mem::take(&mut state.target_specs));
    let shared_sysroots = RwLock::new(std::mem::take(&mut state.sysroots));
    let shared_config_rlibs = RwLock::new(std::mem::take(&mut state.config_rlibs));

    // Clone config for worker threads. state.config stays populated for main-thread use.
    let config = state.config.clone();
    let boot_binary_name = config.profile.boot_binary.clone();

    // Channels for crate compilation jobs.
    let (job_tx, job_rx) = mpsc::channel::<CompileJob>();
    let (result_tx, result_rx) = mpsc::channel::<CompileOutcome>();
    let job_rx = Mutex::new(job_rx);

    // References for workers.
    let all_crates_ref = &all_crates;
    let config_ref = &config;
    let shared_target_specs_ref = &shared_target_specs;
    let shared_sysroots_ref = &shared_sysroots;
    let shared_config_rlibs_ref = &shared_config_rlibs;
    let shared_artifacts_ref = &shared_artifacts;
    let job_rx_ref = &job_rx;

    let dag_result: Result<(Vec<(String, Duration)>, Duration)> = std::thread::scope(|s| {
        // Spawn worker threads for crate compilation.
        for _ in 0..num_workers {
            let tx = result_tx.clone();
            s.spawn(move || {
                loop {
                    let job = match job_rx_ref.lock().unwrap().recv() {
                        Ok(j) => j,
                        Err(_) => break,
                    };

                    let (krate, _) = &all_crates_ref[job.krate_idx];

                    let specs = shared_target_specs_ref.read().unwrap();
                    let target_spec = specs.get(&krate.target).cloned();
                    drop(specs);

                    let sysroots = shared_sysroots_ref.read().unwrap();
                    let sysroot_dir = sysroots.get(&krate.target).cloned();
                    drop(sysroots);

                    let config_rlib = if job.has_config {
                        let rlibs = shared_config_rlibs_ref.read().unwrap();
                        rlibs.get(&krate.target).cloned()
                    } else {
                        None
                    };

                    let arts = shared_artifacts_ref.read().unwrap();
                    let start = Instant::now();
                    let result = compile::compile_crate(
                        krate,
                        config_ref,
                        target_spec.as_deref(),
                        sysroot_dir.as_deref(),
                        &arts,
                        config_rlib.as_deref(),
                        None,
                        job.mode,
                        &[],
                    );
                    let elapsed = start.elapsed();
                    drop(arts);

                    let outcome = match result {
                        Ok(artifact) => CompileOutcome::Compiled {
                            node_idx: job.node_idx,
                            artifact,
                            flags_hash: job.flags_hash,
                            duration: elapsed,
                        },
                        Err(error) => CompileOutcome::Error {
                            node_idx: job.node_idx,
                            error,
                        },
                    };
                    if tx.send(outcome).is_err() {
                        break;
                    }
                }
            });
        }

        // Drop the cloned sender so the channel closes when workers finish.
        drop(result_tx);

        // --- Main thread: process DAG nodes ---
        let mut completed_count = 0usize;
        let mut in_flight = 0usize;
        let mut crate_timings: Vec<(String, Duration)> = Vec::new();
        let pipeline_start = Instant::now();

        while completed_count < total {
            // Process all ready nodes.
            let batch: Vec<usize> = ready_queue.drain(..).collect();
            for idx in batch {
                match &nodes[idx] {
                    DagNode::Sysroot { target } => {
                        // Build sysroot on main thread using brief RwLock ops.
                        ensure_sysroot(
                            model,
                            &config,
                            &mut state.cache,
                            state.force,
                            target,
                            &root,
                            &shared_target_specs,
                            &shared_sysroots,
                        )?;

                        completed_count += 1;
                        for &dep_idx in &dependents[idx] {
                            in_degree[dep_idx] -= 1;
                            if in_degree[dep_idx] == 0 {
                                ready_queue.push(dep_idx);
                            }
                        }
                    }
                    DagNode::ConfigCrate { target } => {
                        // Build config crate on main thread using brief RwLock ops.
                        ensure_config_crate(
                            &config,
                            &mut state.cache,
                            &mut state.rebuilt,
                            state.force,
                            target,
                            &shared_target_specs,
                            &shared_sysroots,
                            &shared_config_rlibs,
                            &shared_artifacts,
                        )?;

                        completed_count += 1;
                        for &dep_idx in &dependents[idx] {
                            in_degree[dep_idx] -= 1;
                            if in_degree[dep_idx] == 0 {
                                ready_queue.push(dep_idx);
                            }
                        }
                    }
                    DagNode::Rule { name } => {
                        // Execute rules on main thread using brief RwLock ops.
                        execute_rule(
                            model,
                            &config,
                            name,
                            &root,
                            &mut state.kernel_binary,
                            state.kernel_binary_rebuilt,
                            state.force,
                            &mut state.cache,
                            &shared_artifacts,
                            &shared_target_specs,
                            &shared_sysroots,
                            &shared_config_rlibs,
                        )?;

                        completed_count += 1;
                        for &dep_idx in &dependents[idx] {
                            in_degree[dep_idx] -= 1;
                            if in_degree[dep_idx] == 0 {
                                ready_queue.push(dep_idx);
                            }
                        }
                    }
                    DagNode::Crate {
                        krate_idx,
                        has_config,
                    } => {
                        let (krate, _) = &all_crates[*krate_idx];
                        let is_host = krate.target == "host";
                        let artifact_path = compile::crate_artifact_path(krate, &root, None, mode);
                        let dep_names: Vec<String> =
                            krate.deps.iter().map(|d| d.crate_name.clone()).collect();

                        let mode_tag = match mode {
                            CompileMode::Build if is_host => "host",
                            CompileMode::Build => "kernel",
                            CompileMode::Check => "check",
                            CompileMode::Clippy => "clippy",
                        };

                        let flags_hash = {
                            let rustc_flags_str = krate.rustc_flags.join(",");
                            let features_str = krate.features.join(",");
                            let cfg_flags_str = krate.cfg_flags.join(",");
                            let linker_script_str = krate.linker_script.as_deref().unwrap_or("");

                            if is_host {
                                compile::hash_args(&[
                                    "host".as_ref(),
                                    krate.name.as_ref(),
                                    krate.edition.as_ref(),
                                    krate.crate_type.as_str().as_ref(),
                                    rustc_flags_str.as_ref(),
                                    features_str.as_ref(),
                                    cfg_flags_str.as_ref(),
                                    linker_script_str.as_ref(),
                                ])
                            } else {
                                let specs = shared_target_specs.read().unwrap();
                                let target_spec =
                                    specs.get(&krate.target).map(|s| s.as_str()).unwrap_or("");
                                let hash = compile::hash_args(&[
                                    mode_tag.as_ref(),
                                    krate.name.as_ref(),
                                    krate.edition.as_ref(),
                                    krate.crate_type.as_str().as_ref(),
                                    format!("{}", config.profile.opt_level).as_ref(),
                                    target_spec.as_ref(),
                                    rustc_flags_str.as_ref(),
                                    features_str.as_ref(),
                                    cfg_flags_str.as_ref(),
                                    linker_script_str.as_ref(),
                                ]);
                                drop(specs);
                                hash
                            }
                        };

                        // Check cache freshness (main thread only — mutates cache).
                        if !state.force {
                            if let Some(entry) = state.cache.entries.get_mut(&krate.name) {
                                let freshness =
                                    entry.is_fresh(&flags_hash, &state.rebuilt, &dep_names);
                                if freshness.is_fresh() {
                                    crate::verbose::vprintln!(
                                        "  Skipping {} (unchanged)",
                                        krate.name
                                    );
                                    if krate.name == boot_binary_name {
                                        state.kernel_binary = Some(artifact_path.clone());
                                    }
                                    shared_artifacts
                                        .write()
                                        .unwrap()
                                        .insert(&krate.name, artifact_path);
                                    completed_count += 1;

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

                        // Dispatch to worker.
                        let verb = match mode {
                            CompileMode::Build => "Compiling",
                            CompileMode::Check => "Checking",
                            CompileMode::Clippy => "Checking",
                        };
                        let ctx_tag = if is_host { " (host)" } else { "" };
                        crate::verbose::dprintln!("  {verb} {}{}...", krate.name, ctx_tag);

                        let _ = job_tx.send(CompileJob {
                            node_idx: idx,
                            krate_idx: *krate_idx,
                            flags_hash,
                            has_config: *has_config,
                            mode,
                        });
                        in_flight += 1;
                    }
                }
            }

            // If nothing in flight and nothing ready, check state.
            if in_flight == 0 {
                if completed_count >= total {
                    break;
                }
                if ready_queue.is_empty() {
                    bail!(
                        "dependency cycle detected: {} of {} nodes cannot be scheduled",
                        total - completed_count,
                        total,
                    );
                }
                continue;
            }

            // Wait for one compilation result.
            match result_rx.recv() {
                Ok(CompileOutcome::Compiled {
                    node_idx,
                    artifact,
                    flags_hash,
                    duration,
                }) => {
                    in_flight -= 1;
                    completed_count += 1;

                    let krate_idx = match &nodes[node_idx] {
                        DagNode::Crate { krate_idx, .. } => *krate_idx,
                        _ => unreachable!("CompileOutcome from non-Crate node"),
                    };
                    let (krate, _) = &all_crates[krate_idx];
                    crate_timings.push((krate.name.clone(), duration));
                    let dep_info_path = compile::crate_dep_info_path(krate, &root, None);

                    // Update cache.
                    if let Ok(entry) =
                        CrateEntry::from_compilation(flags_hash, &artifact, &dep_info_path)
                    {
                        state.cache.entries.insert(krate.name.clone(), entry);
                    }

                    // Track kernel binary.
                    if krate.name == boot_binary_name {
                        state.kernel_binary = Some(artifact.clone());
                        state.kernel_binary_rebuilt = true;
                    }

                    state.rebuilt.insert(krate.name.clone());
                    shared_artifacts
                        .write()
                        .unwrap()
                        .insert(&krate.name, artifact);
                    state.recompiled_crates += 1;

                    for &dep_idx in &dependents[node_idx] {
                        in_degree[dep_idx] -= 1;
                        if in_degree[dep_idx] == 0 {
                            ready_queue.push(dep_idx);
                        }
                    }
                }
                Ok(CompileOutcome::Error { node_idx, error }) => {
                    in_flight -= 1;
                    let krate_name = match &nodes[node_idx] {
                        DagNode::Crate { krate_idx, .. } => &all_crates[*krate_idx].0.name,
                        _ => unreachable!(),
                    };
                    let err = anyhow::anyhow!("failed to compile '{}': {error}", krate_name);
                    // Close job channel and drain remaining.
                    drop(job_tx);
                    while in_flight > 0 {
                        if result_rx.recv().is_ok() {
                            in_flight -= 1;
                        } else {
                            break;
                        }
                    }
                    return Err(err);
                }
                Err(_) => {
                    bail!("worker threads terminated unexpectedly");
                }
            }
        }

        // Close job channel to shut down workers.
        drop(job_tx);
        Ok((crate_timings, pipeline_start.elapsed()))
    });

    // Move state back from RwLocks.
    state.artifacts = shared_artifacts.into_inner().unwrap();
    state.target_specs = shared_target_specs.into_inner().unwrap();
    state.sysroots = shared_sysroots.into_inner().unwrap();
    state.config_rlibs = shared_config_rlibs.into_inner().unwrap();

    let (crate_timings, total_elapsed) = dag_result?;

    // Print timing summary if any crates were compiled.
    if !crate_timings.is_empty() {
        let top_n = if crate::verbose::is_verbose() { 5 } else { 3 };
        let mut sorted = crate_timings;
        sorted.sort_by(|a, b| b.1.cmp(&a.1));
        sorted.truncate(top_n);

        crate::verbose::dprintln!("");
        crate::verbose::dprintln!("  Slowest crates:");
        for (name, dur) in &sorted {
            crate::verbose::dprintln!("    {:<24} {:.1?}", name, dur);
        }
    }

    state.pipeline_elapsed = Some(total_elapsed);

    Ok(())
}

// ---------------------------------------------------------------------------
// Sysroot helper (executed on main thread, uses brief RwLock ops)
// ---------------------------------------------------------------------------

/// Ensure a sysroot exists for the given target, building it lazily if needed.
fn ensure_sysroot(
    model: &BuildModel,
    config: &ResolvedConfig,
    cache: &mut CacheManifest,
    force: bool,
    target: &str,
    root: &Path,
    shared_target_specs: &RwLock<HashMap<String, String>>,
    shared_sysroots: &RwLock<HashMap<String, PathBuf>>,
) -> Result<()> {
    if shared_sysroots.read().unwrap().contains_key(target) {
        return Ok(());
    }

    let target_def = model
        .targets
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("target '{target}' not found in model"))?;

    // Builtin targets use rustc's own sysroot — no custom sysroot build needed.
    if target_def.builtin {
        let target_spec = target_def.spec.clone();
        shared_target_specs
            .write()
            .unwrap()
            .insert(target.to_string(), target_spec);
        // Use the default rustc sysroot for builtin targets.
        let default_sysroot = crate::rustc_info::sysroot_path().clone();
        shared_sysroots
            .write()
            .unwrap()
            .insert(target.to_string(), default_sysroot);
        return Ok(());
    }

    let target_spec_path = root.join(&target_def.spec);
    let target_spec = target_spec_path
        .to_str()
        .expect("target spec path is valid UTF-8")
        .to_string();

    // Store the spec path for this target (brief write lock).
    shared_target_specs
        .write()
        .unwrap()
        .insert(target.to_string(), target_spec);

    // Build or reuse sysroot.
    crate::verbose::dprintln!("  Building sysroot for {target}...");
    let sysroot_src = sysroot::sysroot_src_dir()?;
    let sources_hash = crate::cache::hash_sysroot_sources(&sysroot_src);
    let target_spec_file_hash = crate::cache::hash_file(&target_spec_path).unwrap_or_default();
    let sysroot_dir = if !force
        && cache
            .is_sysroot_fresh(
                target,
                config.profile.opt_level,
                &sources_hash,
                Some(&target_spec_file_hash),
            )
            .is_fresh()
    {
        crate::verbose::vprintln!("  Sysroot unchanged, skipping.");
        sysroot::sysroot_output_paths(root, target).sysroot_dir
    } else {
        let sysroot_output =
            sysroot::build_sysroot(root, &target_spec_path, target, config.profile.opt_level)?;
        cache.record_sysroot(
            target,
            config.profile.opt_level,
            sysroot_output.core_rlib,
            sysroot_output.compiler_builtins_rlib,
            sysroot_output.alloc_rlib,
            sources_hash,
            target_spec_file_hash,
        );
        crate::verbose::dprintln!("  Sysroot ready.");
        sysroot_output.sysroot_dir
    };

    // Record sysroot (brief write lock).
    shared_sysroots
        .write()
        .unwrap()
        .insert(target.to_string(), sysroot_dir);
    Ok(())
}

// ---------------------------------------------------------------------------
// Config crate helper (executed on main thread, uses brief RwLock ops)
// ---------------------------------------------------------------------------

/// Ensure a config crate exists for the given target, building it lazily if needed.
fn ensure_config_crate(
    config: &ResolvedConfig,
    cache: &mut CacheManifest,
    rebuilt: &mut HashSet<String>,
    force: bool,
    target: &str,
    shared_target_specs: &RwLock<HashMap<String, String>>,
    shared_sysroots: &RwLock<HashMap<String, PathBuf>>,
    shared_config_rlibs: &RwLock<HashMap<String, PathBuf>>,
    shared_artifacts: &RwLock<ArtifactMap>,
) -> Result<()> {
    if shared_config_rlibs.read().unwrap().contains_key(target) {
        return Ok(());
    }

    let target_spec = shared_target_specs
        .read()
        .unwrap()
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("target spec for '{target}' not resolved"))?
        .clone();
    let sysroot_dir = shared_sysroots
        .read()
        .unwrap()
        .get(target)
        .ok_or_else(|| anyhow::anyhow!("sysroot for '{target}' not built"))?
        .clone();

    crate::verbose::dprintln!("  Generating hadron_config for {target}...");
    let config_dep_info = compile::config_crate_dep_info_path(config);

    let config_flags_hash = {
        let mut parts: Vec<&std::ffi::OsStr> = vec!["hadron_config".as_ref()];
        let opt_str = format!("{}", config.profile.opt_level);
        parts.push(opt_str.as_ref());
        parts.push(target_spec.as_ref());
        compile::hash_args(&parts)
    };

    let config_rlib_path = config
        .root
        .join("build/kernel")
        .join(target)
        .join("debug/libhadron_config.rlib");

    let config_needs_rebuild = force || {
        let key = format!("hadron_config_{target}");
        match cache.entries.get_mut(&key) {
            Some(entry) => !entry.is_fresh(&config_flags_hash, rebuilt, &[]).is_fresh(),
            None => true,
        }
    };

    let config_rlib = if config_needs_rebuild {
        let key = format!("hadron_config_{target}");
        // Snapshot the old artifact hash before rebuilding.
        let old_entry_unchanged = cache
            .entries
            .get(&key)
            .map(|e| e.artifact_content_unchanged())
            .unwrap_or(false);
        let old_hash = cache
            .entries
            .get(&key)
            .and_then(|e| e.artifact_hash.clone());

        let rlib = compile::build_config_crate(config, &target_spec, &sysroot_dir)?;
        let new_hash = crate::cache::hash_file(&rlib).ok();

        if let Ok(mut entry) =
            CrateEntry::from_compilation(config_flags_hash, &rlib, &config_dep_info)
        {
            entry.artifact_hash = new_hash.clone();
            cache.entries.insert(key, entry);
        }

        // Only mark as rebuilt if the artifact binary actually changed.
        let content_changed = match (&old_hash, &new_hash) {
            (Some(old), Some(new)) if old_entry_unchanged => old != new,
            _ => true, // No previous hash or file was already stale — assume changed.
        };
        if content_changed {
            rebuilt.insert(format!("hadron_config_{target}"));
        } else {
            crate::verbose::vprintln!(
                "  hadron_config binary unchanged, skipping cascade for {target}"
            );
        }
        rlib
    } else {
        crate::verbose::vprintln!("  Skipping hadron_config (unchanged)");
        config_rlib_path
    };

    // Brief write locks for the inserts.
    shared_artifacts
        .write()
        .unwrap()
        .insert(&format!("hadron_config_{target}"), config_rlib.clone());
    shared_config_rlibs
        .write()
        .unwrap()
        .insert(target.to_string(), config_rlib);

    Ok(())
}

// ---------------------------------------------------------------------------
// Rule execution (main thread, uses brief RwLock ops)
// ---------------------------------------------------------------------------

/// Execute a named rule.
fn execute_rule(
    model: &BuildModel,
    config: &ResolvedConfig,
    rule_name: &str,
    root: &Path,
    kernel_binary: &mut Option<PathBuf>,
    kernel_binary_rebuilt: bool,
    force: bool,
    cache: &mut CacheManifest,
    shared_artifacts: &RwLock<ArtifactMap>,
    shared_target_specs: &RwLock<HashMap<String, String>>,
    shared_sysroots: &RwLock<HashMap<String, PathBuf>>,
    shared_config_rlibs: &RwLock<HashMap<String, PathBuf>>,
) -> Result<()> {
    let rule = model
        .rules
        .get(rule_name)
        .ok_or_else(|| anyhow::anyhow!("rule '{rule_name}' not found"))?;

    match &rule.handler {
        RuleHandler::Builtin(handler_name) => {
            match handler_name.as_str() {
                "hbtf" => {
                    if let Some(kernel_bin) = kernel_binary.as_ref() {
                        if kernel_binary_rebuilt || force {
                            let hbtf_path = root.join("build/backtrace.hbtf");
                            println!("\nGenerating HBTF...");
                            crate::artifact::hbtf::generate_hbtf(
                                kernel_bin,
                                &hbtf_path,
                                config.profile.debug_info,
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

                    // Collect binary artifacts from the rule's inputs (brief read lock).
                    let mut bin_artifacts = Vec::new();
                    {
                        let arts = shared_artifacts.read().unwrap();
                        for input in &rule.inputs {
                            if let Some(path) = arts.get(input) {
                                if let Some(def) = model.crates.get(input) {
                                    if def.crate_type == crate::model::CrateType::Bin {
                                        bin_artifacts.push((input.clone(), path.to_path_buf()));
                                    }
                                }
                            }
                        }
                    }

                    let initrd_fresh = !force
                        && cache.is_initrd_fresh(&initrd_path, &bin_artifacts)
                        && target_initrd.exists();

                    if initrd_fresh {
                        println!("\nInitrd unchanged, skipping.");
                    } else {
                        println!("\nBuilding initrd...");
                        let built = crate::artifact::initrd::build_initrd(config, &bin_artifacts)?;
                        cache.record_initrd(&built, &bin_artifacts);
                        cache.save(&config.root)?;

                        std::fs::create_dir_all(target_initrd.parent().unwrap())?;
                        std::fs::copy(&built, &target_initrd)?;
                    }
                }
                "hkif" => {
                    if let Some(kernel_bin) = kernel_binary.as_ref() {
                        if kernel_binary_rebuilt || force {
                            println!("\nGenerating HKIF (two-pass link)...");
                            let hkif_bin = root.join("build/hkif.bin");
                            let hkif_asm = root.join("build/hkif.S");
                            let hkif_obj = crate::artifact::hkif::hkif_object_path(root);

                            crate::artifact::hkif::generate_hkif(
                                kernel_bin,
                                &hkif_bin,
                                config.profile.debug_info,
                            )?;
                            crate::artifact::hkif::generate_hkif_asm(&hkif_bin, &hkif_asm)?;
                            crate::artifact::hkif::assemble_hkif(&hkif_asm, &hkif_obj)?;

                            let boot_name = &config.profile.boot_binary;
                            let sysroot_src = sysroot::sysroot_src_dir()?;
                            let group_crates = crate_graph::resolve_group_from_model(
                                model,
                                "kernel-main",
                                root,
                                &sysroot_src,
                            )?;
                            let boot_crate = group_crates
                                .iter()
                                .find(|k| k.name == *boot_name)
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "boot binary '{}' not found in kernel-main group",
                                        boot_name
                                    )
                                })?;

                            // Brief read locks for target spec, sysroot, config rlib.
                            let target_spec = shared_target_specs
                                .read()
                                .unwrap()
                                .get(&boot_crate.target)
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "target spec for '{}' not found",
                                        boot_crate.target
                                    )
                                })?
                                .clone();
                            let sysroot_dir = shared_sysroots
                                .read()
                                .unwrap()
                                .get(&boot_crate.target)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("sysroot for '{}' not found", boot_crate.target)
                                })?
                                .clone();
                            let config_rlib = shared_config_rlibs
                                .read()
                                .unwrap()
                                .get(&boot_crate.target)
                                .cloned();

                            println!("  Re-linking {} with HKIF object...", boot_name);
                            // Hold read lock on artifacts during relink (workers can
                            // also hold read locks concurrently — this is safe).
                            let arts = shared_artifacts.read().unwrap();
                            let new_binary = compile::relink_with_objects(
                                boot_crate,
                                config,
                                &target_spec,
                                &sysroot_dir,
                                &arts,
                                config_rlib.as_deref(),
                                &[hkif_obj],
                                &[],
                                None,
                            )?;
                            drop(arts);

                            *kernel_binary = Some(new_binary.clone());
                            shared_artifacts
                                .write()
                                .unwrap()
                                .insert(boot_name, new_binary);
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
        RuleHandler::Script(source) => {
            let ctx = crate::rule_engine::RuleContext::new(
                rule_name,
                rule,
                config,
                root,
                kernel_binary,
                kernel_binary_rebuilt,
                force,
                shared_artifacts,
            );
            let engine = crate::rule_engine::create_rule_engine(
                model,
                config,
                shared_artifacts,
                shared_target_specs,
                shared_sysroots,
                shared_config_rlibs,
            );
            let ctx = crate::rule_engine::execute_script(&engine, source, rule_name, ctx)?;

            // Apply mutations from the script.
            if let Some(new_kb) = ctx.kernel_binary_override {
                *kernel_binary = Some(PathBuf::from(&new_kb));
                shared_artifacts
                    .write()
                    .unwrap()
                    .insert(&config.profile.boot_binary, PathBuf::from(&new_kb));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// DAG construction helper (extracted for testing)
// ---------------------------------------------------------------------------

/// Result of building a DAG: node indices, edges, and in-degrees.
#[cfg(test)]
struct DagInfo {
    node_name_to_idx: HashMap<String, usize>,
    in_degree: Vec<usize>,
    edge_set: HashSet<(usize, usize)>,
}

/// Build a DAG from a model without executing it. Used by tests to verify
/// edge creation for barriers, artifact deps, and deduplication.
#[cfg(test)]
fn build_dag(model: &crate::model::BuildModel) -> DagInfo {
    use crate::model::PipelineStep;

    // Resolve all crates (host-only for test models, no sysroot needed).
    let root = std::path::Path::new("/fake/root");
    let sysroot_src = std::path::Path::new("/fake/sysroot");
    let config_options = std::collections::BTreeMap::new();
    let all_crates = crate_graph::resolve_all_groups(model, root, sysroot_src, &config_options)
        .expect("resolve_all_groups should succeed in test");

    let mut nodes: Vec<String> = Vec::new();
    let mut node_name_to_idx: HashMap<String, usize> = HashMap::new();

    // Add Crate nodes.
    for (krate, _) in &all_crates {
        let idx = nodes.len();
        node_name_to_idx.insert(krate.name.clone(), idx);
        nodes.push(krate.name.clone());
    }

    // Add Rule nodes.
    let rules: Vec<String> = model
        .pipeline
        .steps
        .iter()
        .filter_map(|step| {
            if let PipelineStep::Rule(name) = step {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();
    for rule_name in &rules {
        let idx = nodes.len();
        node_name_to_idx.insert(format!("__rule_{rule_name}"), idx);
        nodes.push(format!("__rule_{rule_name}"));
    }

    let total = nodes.len();
    let mut in_degree: Vec<usize> = vec![0; total];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); total];
    let mut edge_set: HashSet<(usize, usize)> = HashSet::new();

    let add_edge = |from: usize,
                    to: usize,
                    edge_set: &mut HashSet<(usize, usize)>,
                    in_degree: &mut Vec<usize>,
                    dependents: &mut Vec<Vec<usize>>| {
        if edge_set.insert((from, to)) {
            in_degree[to] += 1;
            dependents[from].push(to);
        }
    };

    // Crate-to-crate edges from resolved dependencies.
    for (krate, _) in &all_crates {
        let crate_node_idx = *node_name_to_idx.get(&krate.name).unwrap();
        for dep in &krate.deps {
            if let Some(&dep_node_idx) = node_name_to_idx.get(&dep.crate_name) {
                add_edge(
                    dep_node_idx,
                    crate_node_idx,
                    &mut edge_set,
                    &mut in_degree,
                    &mut dependents,
                );
            }
        }
    }

    // Rule edges.
    for rule_name in &rules {
        let rule_node_idx = *node_name_to_idx
            .get(&format!("__rule_{rule_name}"))
            .unwrap();
        if let Some(rule) = model.rules.get(rule_name) {
            for input in &rule.inputs {
                if let Some(&input_idx) = node_name_to_idx.get(input) {
                    add_edge(
                        input_idx,
                        rule_node_idx,
                        &mut edge_set,
                        &mut in_degree,
                        &mut dependents,
                    );
                }
            }
            for dep_rule in &rule.depends_on {
                if let Some(&dep_idx) = node_name_to_idx.get(&format!("__rule_{dep_rule}")) {
                    add_edge(
                        dep_idx,
                        rule_node_idx,
                        &mut edge_set,
                        &mut in_degree,
                        &mut dependents,
                    );
                }
            }
        }
    }

    // Barrier edges.
    {
        let mut segments: Vec<Vec<usize>> = Vec::new();
        let mut current_segment: Vec<usize> = Vec::new();

        for step in &model.pipeline.steps {
            match step {
                PipelineStep::Stage { groups, .. } => {
                    for gname in groups {
                        if let Some(group) = model.groups.get(gname) {
                            for crate_name in &group.crates {
                                if let Some(&idx) = node_name_to_idx.get(crate_name) {
                                    current_segment.push(idx);
                                }
                            }
                        }
                    }
                }
                PipelineStep::Rule(rule_name) => {
                    if let Some(&idx) = node_name_to_idx.get(&format!("__rule_{rule_name}")) {
                        current_segment.push(idx);
                    }
                }
                PipelineStep::Barrier(_) => {
                    if !current_segment.is_empty() {
                        segments.push(std::mem::take(&mut current_segment));
                    }
                }
            }
        }
        if !current_segment.is_empty() {
            segments.push(current_segment);
        }

        for i in 0..segments.len().saturating_sub(1) {
            for &pre in &segments[i] {
                for &post in &segments[i + 1] {
                    add_edge(pre, post, &mut edge_set, &mut in_degree, &mut dependents);
                }
            }
        }
    }

    // Artifact dependency edges.
    for (krate, _) in &all_crates {
        if let Some(def) = model.crates.get(&krate.name) {
            let crate_idx = *node_name_to_idx.get(&krate.name).unwrap();
            for art_dep in &def.artifact_deps {
                if let Some(&dep_idx) = node_name_to_idx
                    .get(art_dep)
                    .or_else(|| node_name_to_idx.get(&format!("__rule_{art_dep}")))
                {
                    add_edge(
                        dep_idx,
                        crate_idx,
                        &mut edge_set,
                        &mut in_degree,
                        &mut dependents,
                    );
                }
            }
        }
    }

    DagInfo {
        node_name_to_idx,
        in_degree,
        edge_set,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        BuildModel, CrateDef, CrateType, DepDef, GroupDef, PipelineDef, PipelineStep, RuleDef,
        RuleHandler,
    };
    use std::collections::BTreeMap;

    /// Build a minimal model with groups and a pipeline for DAG testing.
    /// Each entry is (group_name, [(crate_name, [dep_names])]).
    /// Pipeline steps alternate: stage(group), barrier, stage(group), ...
    fn make_pipeline_model(groups: &[(&str, &[(&str, &[&str])])], barriers: &[&str]) -> BuildModel {
        let mut model = BuildModel::default();
        model.project.name = "test".into();

        let mut steps: Vec<PipelineStep> = Vec::new();
        let mut barrier_idx = 0;

        for (gname, crates) in groups {
            let mut group = GroupDef::default();
            group.name = (*gname).into();

            for &(name, deps) in *crates {
                group.crates.push(name.into());

                let mut dep_map = BTreeMap::new();
                for &dep in deps {
                    dep_map.insert(
                        dep.into(),
                        DepDef {
                            extern_name: dep.into(),
                            crate_name: dep.into(),
                            features: vec![],
                            version: None,
                        },
                    );
                }

                model.crates.insert(
                    name.into(),
                    CrateDef {
                        name: name.into(),
                        path: format!("crates/{name}"),
                        edition: "2024".into(),
                        crate_type: CrateType::Lib,
                        target: "host".into(),
                        deps: dep_map,
                        dev_deps: BTreeMap::new(),
                        features: vec![],
                        root: None,
                        linker_script: None,
                        group: Some((*gname).into()),
                        is_project_crate: true,
                        cfg_flags: vec![],
                        rustc_flags: vec![],
                        requires_config: vec![],
                        artifact_deps: vec![],
                    },
                );
            }

            model.groups.insert((*gname).into(), group);

            steps.push(PipelineStep::Stage {
                name: (*gname).into(),
                groups: vec![(*gname).into()],
            });

            if barrier_idx < barriers.len() {
                steps.push(PipelineStep::Barrier(barriers[barrier_idx].into()));
                barrier_idx += 1;
            }
        }

        model.pipeline = PipelineDef { steps };
        model
    }

    #[test]
    fn test_barrier_creates_edges() {
        // Two stages separated by a barrier: stage1=[a, b], barrier, stage2=[c, d]
        let model = make_pipeline_model(
            &[
                ("stage1", &[("a", &[]), ("b", &[])]),
                ("stage2", &[("c", &[]), ("d", &[])]),
            ],
            &["barrier1"],
        );

        let dag = build_dag(&model);
        let a = dag.node_name_to_idx["a"];
        let b = dag.node_name_to_idx["b"];
        let c = dag.node_name_to_idx["c"];
        let d = dag.node_name_to_idx["d"];

        // All pre-barrier crates should have edges to all post-barrier crates.
        assert!(dag.edge_set.contains(&(a, c)), "expected edge a->c");
        assert!(dag.edge_set.contains(&(a, d)), "expected edge a->d");
        assert!(dag.edge_set.contains(&(b, c)), "expected edge b->c");
        assert!(dag.edge_set.contains(&(b, d)), "expected edge b->d");

        // Post-barrier crates should have in-degree of 2 (from a and b).
        assert_eq!(dag.in_degree[c], 2);
        assert_eq!(dag.in_degree[d], 2);
    }

    #[test]
    fn test_artifact_deps_create_edges() {
        // Crate B has artifact_deps on A, but no Rust dep.
        let mut model = make_pipeline_model(&[("stage1", &[("a", &[]), ("b", &[])])], &[]);
        model.crates.get_mut("b").unwrap().artifact_deps = vec!["a".into()];

        let dag = build_dag(&model);
        let a = dag.node_name_to_idx["a"];
        let b = dag.node_name_to_idx["b"];

        assert!(
            dag.edge_set.contains(&(a, b)),
            "expected artifact dep edge a->b"
        );
        assert_eq!(dag.in_degree[b], 1);
    }

    #[test]
    fn test_edge_deduplication() {
        // Crate B has both a Rust dep on A and an artifact dep on A.
        // Should result in only one edge (in_degree = 1, not 2).
        let mut model = make_pipeline_model(&[("stage1", &[("a", &[]), ("b", &["a"])])], &[]);
        model.crates.get_mut("b").unwrap().artifact_deps = vec!["a".into()];

        let dag = build_dag(&model);
        let b = dag.node_name_to_idx["b"];

        assert_eq!(
            dag.in_degree[b], 1,
            "edge should be deduplicated: in_degree should be 1, not 2"
        );
    }

    #[test]
    fn test_barrier_transitivity() {
        // Three stages: A | B | C (two barriers).
        // C should depend on B (direct), A should depend on nothing,
        // B should depend on A. No direct A->C edge.
        let model = make_pipeline_model(
            &[
                ("stage1", &[("a", &[])]),
                ("stage2", &[("b", &[])]),
                ("stage3", &[("c", &[])]),
            ],
            &["barrier1", "barrier2"],
        );

        let dag = build_dag(&model);
        let a = dag.node_name_to_idx["a"];
        let b = dag.node_name_to_idx["b"];
        let c = dag.node_name_to_idx["c"];

        // Direct edges: a->b (barrier1) and b->c (barrier2).
        assert!(dag.edge_set.contains(&(a, b)), "expected edge a->b");
        assert!(dag.edge_set.contains(&(b, c)), "expected edge b->c");

        // No direct a->c edge (transitivity through b handles this).
        assert!(
            !dag.edge_set.contains(&(a, c)),
            "should NOT have direct edge a->c"
        );

        assert_eq!(dag.in_degree[a], 0);
        assert_eq!(dag.in_degree[b], 1);
        assert_eq!(dag.in_degree[c], 1);
    }
}
