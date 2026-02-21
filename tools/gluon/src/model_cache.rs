//! Model cache for skipping script re-evaluation on incremental builds.
//!
//! Serializes the [`BuildModel`] to `build/model-cache.json` alongside an
//! envelope of input file mtimes. On subsequent runs, if all input mtimes
//! match, the cached model is deserialized directly — skipping Rhai
//! evaluation, Kconfig parsing, and vendor resolution.
//!
//! Uses a `.lock` sentinel file with exclusive-create semantics to prevent
//! concurrent processes from corrupting the cache.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::cache::file_mtime_secs;
use crate::model::BuildModel;

/// Cache filename within the build directory.
const MODEL_CACHE_FILE: &str = "model-cache.json";

/// Lock filename within the build directory.
const MODEL_LOCK_FILE: &str = "model-cache.lock";

/// Maximum age of a lock file before it is considered stale (seconds).
const STALE_LOCK_SECS: u64 = 60;

/// Envelope wrapping the cached model with input file mtimes.
#[derive(Serialize, Deserialize)]
struct ModelCacheEnvelope {
    /// Mtimes of all input files at the time the model was cached.
    input_mtimes: HashMap<PathBuf, i64>,
    /// The cached build model.
    model: BuildModel,
}

/// RAII guard for the model cache lock file.
///
/// Removes the lock file on drop.
struct CacheLockGuard {
    lock_path: PathBuf,
}

impl Drop for CacheLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Try to acquire an exclusive lock by creating a sentinel file with
/// `O_CREAT | O_EXCL` semantics (fails if file already exists).
///
/// Returns `Some(guard)` on success, `None` if the lock is held by another
/// process (or the lock file is stale and was cleaned up — caller should retry).
fn try_acquire_lock(build_dir: &Path) -> Option<CacheLockGuard> {
    let lock_path = build_dir.join(MODEL_LOCK_FILE);

    // Clean stale locks older than STALE_LOCK_SECS.
    if lock_path.exists() {
        if let Ok(meta) = std::fs::metadata(&lock_path) {
            if let Ok(modified) = meta.modified() {
                let age = SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default();
                if age.as_secs() > STALE_LOCK_SECS {
                    crate::verbose::vprintln!("  model cache: removing stale lock file");
                    let _ = std::fs::remove_file(&lock_path);
                }
            }
        }
    }

    // Attempt exclusive creation — OpenOptions with create_new = true maps to
    // O_CREAT | O_EXCL on Unix, which is atomic.
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(_file) => Some(CacheLockGuard { lock_path }),
        Err(_) => {
            crate::verbose::vprintln!("  model cache: lock held by another process, skipping cache");
            None
        }
    }
}

/// Attempt to load a cached model from `build/model-cache.json`.
///
/// Returns `Some(model)` if the cache exists and all tracked input file
/// mtimes match their current values. Returns `None` (with a verbose
/// reason) if the cache is missing, corrupt, stale, or locked.
pub fn load_cached_model(root: &Path) -> Option<BuildModel> {
    let build_dir = root.join("build");
    let lock_path = build_dir.join(MODEL_LOCK_FILE);

    // If the lock file exists, another process is writing — skip cache.
    if lock_path.exists() {
        crate::verbose::vprintln!("  model cache: lock file present, re-evaluating");
        return None;
    }

    let cache_path = build_dir.join(MODEL_CACHE_FILE);
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
/// Acquires an exclusive lock before writing. If the lock cannot be acquired
/// (another process is writing), the save is skipped — the next run will
/// re-evaluate and try again.
pub fn save_cached_model(root: &Path, model: &BuildModel) -> Result<()> {
    let build_dir = root.join("build");
    std::fs::create_dir_all(&build_dir)?;

    let _guard = match try_acquire_lock(&build_dir) {
        Some(g) => g,
        None => {
            // Another process holds the lock — skip saving. Always correct
            // because the next run will re-evaluate the model.
            return Ok(());
        }
    };

    let input_mtimes = collect_model_inputs(root, model);

    let envelope = ModelCacheEnvelope {
        input_mtimes,
        model: model.clone(),
    };

    let cache_path = build_dir.join(MODEL_CACHE_FILE);
    let tmp_path = build_dir.join(format!("{MODEL_CACHE_FILE}.tmp"));

    let json = serde_json::to_string(&envelope)?;
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, &cache_path)?;

    crate::verbose::vprintln!("  model cache: saved");
    // _guard drops here, removing the lock file
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
