//! Model cache for skipping script re-evaluation on incremental builds.
//!
//! Serializes the [`BuildModel`] to `build/model-cache.json` alongside an
//! envelope of input file mtimes. On subsequent runs, if all input mtimes
//! match, the cached model is deserialized directly — skipping Rhai
//! evaluation, Kconfig parsing, and vendor resolution.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::cache::file_mtime_secs;
use crate::model::BuildModel;

/// Cache filename within the build directory.
const MODEL_CACHE_FILE: &str = "model-cache.json";

/// Envelope wrapping the cached model with input file mtimes.
#[derive(Serialize, Deserialize)]
struct ModelCacheEnvelope {
    /// Mtimes of all input files at the time the model was cached.
    input_mtimes: HashMap<PathBuf, i64>,
    /// The cached build model.
    model: BuildModel,
}

/// Attempt to load a cached model from `build/model-cache.json`.
///
/// Returns `Some(model)` if the cache exists and all tracked input file
/// mtimes match their current values. Returns `None` (with a verbose
/// reason) if the cache is missing, corrupt, or stale.
pub fn load_cached_model(root: &Path) -> Option<BuildModel> {
    let cache_path = root.join("build").join(MODEL_CACHE_FILE);
    let data = std::fs::read_to_string(&cache_path).ok()?;

    let envelope: ModelCacheEnvelope = match serde_json::from_str(&data) {
        Ok(e) => e,
        Err(e) => {
            crate::verbose::vprintln!("  model cache: deserialization failed: {e}");
            return None;
        }
    };

    // Verify all input file mtimes still match.
    for (path, cached_mtime) in &envelope.input_mtimes {
        match file_mtime_secs(path) {
            Some(current) if current == *cached_mtime => {}
            Some(_) => {
                crate::verbose::vprintln!("  model cache stale: {} changed", path.display());
                return None;
            }
            None => {
                crate::verbose::vprintln!("  model cache stale: {} missing", path.display());
                return None;
            }
        }
    }

    crate::verbose::vprintln!("  model cache: valid ({} inputs tracked)", envelope.input_mtimes.len());
    Some(envelope.model)
}

/// Save the model to `build/model-cache.json` with current input file mtimes.
///
/// Writes atomically via tmp + rename.
pub fn save_cached_model(root: &Path, model: &BuildModel) -> Result<()> {
    let input_mtimes = collect_model_inputs(root, model);

    let envelope = ModelCacheEnvelope {
        input_mtimes,
        model: model.clone(),
    };

    let build_dir = root.join("build");
    std::fs::create_dir_all(&build_dir)?;

    let cache_path = build_dir.join(MODEL_CACHE_FILE);
    let tmp_path = build_dir.join(format!("{MODEL_CACHE_FILE}.tmp"));

    let json = serde_json::to_string(&envelope)?;
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, &cache_path)?;

    crate::verbose::vprintln!("  model cache: saved");
    Ok(())
}

/// Collect all input files that affect the model, with their current mtimes.
fn collect_model_inputs(root: &Path, model: &BuildModel) -> HashMap<PathBuf, i64> {
    let mut inputs = HashMap::new();

    // Always track gluon.rhai.
    record_mtime(&mut inputs, &root.join("gluon.rhai"));

    // Track gluon.lock if it exists.
    record_mtime(&mut inputs, &root.join("gluon.lock"));

    // Track .hadron-config if it exists.
    record_mtime(&mut inputs, &root.join(".hadron-config"));

    // Track all Kconfig files discovered during evaluation.
    for path in &model.input_files {
        record_mtime(&mut inputs, path);
    }

    // Track vendor/*/Cargo.toml files.
    let vendor_dir = root.join("vendor");
    if let Ok(entries) = std::fs::read_dir(&vendor_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let cargo_toml = entry.path().join("Cargo.toml");
                record_mtime(&mut inputs, &cargo_toml);
            }
        }
    }

    inputs
}

/// Record a file's mtime if it exists.
fn record_mtime(map: &mut HashMap<PathBuf, i64>, path: &Path) {
    if let Some(mtime) = file_mtime_secs(path) {
        map.insert(path.to_path_buf(), mtime);
    }
}
