//! Build cache manifest for skipping unchanged crate compilations.
//!
//! Tracks compiler flags, source file timestamps, and content hashes
//! for each compiled crate. Uses rustc's `.d` dep-info files for precise
//! source dependency tracking with a hybrid mtime + SHA-256 fallback strategy.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Current schema version. Bump when the manifest format changes.
const MANIFEST_VERSION: u32 = 2;

/// Manifest filename within the build directory.
const MANIFEST_FILE: &str = "cache-manifest.json";

/// Result of a freshness check on a cached crate entry.
pub enum FreshResult {
    /// The crate does not need recompilation.
    Fresh,
    /// The crate must be recompiled, with a human-readable reason.
    Stale(String),
}

impl FreshResult {
    /// Returns `true` if the result is `Fresh`.
    pub fn is_fresh(&self) -> bool {
        matches!(self, Self::Fresh)
    }
}

/// Top-level cache manifest tracking all compiled artifacts.
#[derive(Serialize, Deserialize)]
pub struct CacheManifest {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// SHA-256 hash of `rustc -vV` output — detects toolchain changes.
    pub rustc_version_hash: String,
    /// Per-crate cache entries, keyed by crate name.
    pub entries: HashMap<String, CrateEntry>,
    /// Sysroot cache entries, keyed by target name.
    #[serde(default)]
    pub sysroots: HashMap<String, SysrootEntry>,
    /// Initrd output cache entry.
    #[serde(default)]
    pub initrd: Option<InitrdEntry>,
}

/// Cache entry for a sysroot (core, compiler_builtins, alloc) for a given target.
#[derive(Serialize, Deserialize)]
pub struct SysrootEntry {
    /// The optimization level used to build the sysroot.
    pub opt_level: u32,
    /// Paths to the three rlibs — used to verify they still exist.
    pub core_rlib: PathBuf,
    pub compiler_builtins_rlib: PathBuf,
    pub alloc_rlib: PathBuf,
    /// SHA-256 hash of concatenated sysroot source entry points.
    #[serde(default)]
    pub sources_hash: String,
}

/// Cache entry for the initrd output.
#[derive(Serialize, Deserialize)]
pub struct InitrdEntry {
    /// Mtime of the initrd output file.
    pub output_mtime_secs: i64,
    /// Mtimes of userspace crate root source files, for change detection.
    pub source_mtimes: HashMap<PathBuf, i64>,
}

/// Cache entry for a single compiled crate.
#[derive(Serialize, Deserialize)]
pub struct CrateEntry {
    /// SHA-256 hash of the compiler flags used for this crate.
    pub flags_hash: String,
    /// Path to the output artifact (rlib/dylib/bin).
    pub artifact_path: PathBuf,
    /// Artifact file mtime (seconds since epoch).
    pub artifact_mtime_secs: i64,
    /// Source files and their recorded state, from dep-info.
    pub sources: HashMap<PathBuf, SourceRecord>,
    /// SHA-256 of the artifact content (used for config cascade avoidance).
    #[serde(default)]
    pub artifact_hash: Option<String>,
}

/// Recorded state of a single source file dependency.
#[derive(Serialize, Deserialize)]
pub struct SourceRecord {
    /// Last known mtime (seconds since epoch).
    pub mtime_secs: i64,
    /// SHA-256 hash of the file contents.
    pub content_hash: String,
}

impl CacheManifest {
    /// Create a new empty manifest for the current rustc version.
    pub fn new(rustc_version_hash: String) -> Self {
        Self {
            version: MANIFEST_VERSION,
            rustc_version_hash,
            entries: HashMap::new(),
            sysroots: HashMap::new(),
            initrd: None,
        }
    }

    /// Load the manifest from `<root>/build/cache-manifest.json`.
    ///
    /// Returns `None` if the file is missing, corrupt, or has a version mismatch.
    pub fn load(root: &Path) -> Option<Self> {
        let path = root.join("build").join(MANIFEST_FILE);
        let data = fs::read_to_string(&path).ok()?;
        let manifest: Self = serde_json::from_str(&data).ok()?;
        if manifest.version != MANIFEST_VERSION {
            return None;
        }
        Some(manifest)
    }

    /// Save the manifest atomically (write to tmp, then rename).
    pub fn save(&self, root: &Path) -> Result<()> {
        let build_dir = root.join("build");
        fs::create_dir_all(&build_dir)?;

        let path = build_dir.join(MANIFEST_FILE);
        let tmp_path = build_dir.join(format!("{MANIFEST_FILE}.tmp"));

        let json = serde_json::to_string_pretty(self)
            .context("failed to serialize cache manifest")?;
        fs::write(&tmp_path, json)
            .context("failed to write temporary cache manifest")?;
        fs::rename(&tmp_path, &path)
            .context("failed to atomically replace cache manifest")?;

        Ok(())
    }

    /// Check if a sysroot for the given target is still fresh.
    pub fn is_sysroot_fresh(
        &self,
        target_name: &str,
        opt_level: u32,
        current_sources_hash: &str,
    ) -> FreshResult {
        let entry = match self.sysroots.get(target_name) {
            Some(e) => e,
            None => return FreshResult::Stale(format!("no cached sysroot for {target_name}")),
        };

        if entry.opt_level != opt_level {
            return FreshResult::Stale(format!(
                "opt-level changed ({} -> {opt_level})",
                entry.opt_level
            ));
        }

        // Empty stored hash (old manifest) or mismatch triggers rebuild.
        if entry.sources_hash.is_empty() || entry.sources_hash != current_sources_hash {
            return FreshResult::Stale("sysroot sources changed".into());
        }

        for path in [&entry.core_rlib, &entry.compiler_builtins_rlib, &entry.alloc_rlib] {
            if !path.exists() {
                return FreshResult::Stale(format!(
                    "sysroot rlib missing: {}",
                    path.display()
                ));
            }
        }

        FreshResult::Fresh
    }

    /// Record a sysroot build result for a given target.
    pub fn record_sysroot(
        &mut self,
        target_name: &str,
        opt_level: u32,
        core_rlib: PathBuf,
        compiler_builtins_rlib: PathBuf,
        alloc_rlib: PathBuf,
        sources_hash: String,
    ) {
        self.sysroots.insert(
            target_name.to_string(),
            SysrootEntry {
                opt_level,
                core_rlib,
                compiler_builtins_rlib,
                alloc_rlib,
                sources_hash,
            },
        );
    }

    /// Check if the initrd output is still fresh.
    ///
    /// Checks output file existence + mtime, and also that none of the
    /// userspace source root files have changed mtimes.
    pub fn is_initrd_fresh(&self, output_path: &Path, source_roots: &[PathBuf]) -> bool {
        let entry = match &self.initrd {
            Some(e) => e,
            None => return false,
        };

        // Check output file.
        match file_mtime_secs(output_path) {
            Some(mtime) if mtime == entry.output_mtime_secs => {}
            _ => return false,
        }

        // Check each source root file.
        for src in source_roots {
            let current = file_mtime_secs(src);
            let stored = entry.source_mtimes.get(src).copied();
            match (stored, current) {
                (Some(s), Some(c)) if s == c => {}
                _ => return false,
            }
        }

        true
    }

    /// Record a freshly-built initrd in the manifest.
    pub fn record_initrd(&mut self, output_path: &Path, source_roots: &[PathBuf]) {
        let mtime = file_mtime_secs(output_path).unwrap_or(0);
        let mut source_mtimes = HashMap::new();
        for src in source_roots {
            if let Some(m) = file_mtime_secs(src) {
                source_mtimes.insert(src.clone(), m);
            }
        }
        self.initrd = Some(InitrdEntry {
            output_mtime_secs: mtime,
            source_mtimes,
        });
    }
}

impl CrateEntry {
    /// Check if the artifact file is byte-identical to the previously recorded hash.
    ///
    /// Returns `true` if both a stored hash and the current file exist and match.
    pub fn artifact_content_unchanged(&self) -> bool {
        let stored = match &self.artifact_hash {
            Some(h) if !h.is_empty() => h,
            _ => return false,
        };
        match hash_file(&self.artifact_path) {
            Ok(current) => current == *stored,
            Err(_) => false,
        }
    }

    /// Check whether this crate's cached artifact is still fresh.
    ///
    /// `rebuilt_deps` contains the names of crates that were recompiled in this
    /// build session — if any of this crate's dependencies were rebuilt, we must
    /// recompile too.
    ///
    /// `dep_names` is the list of this crate's dependency crate names.
    pub fn is_fresh(
        &mut self,
        flags_hash: &str,
        rebuilt_deps: &HashSet<String>,
        dep_names: &[String],
    ) -> FreshResult {
        // 1. Flags changed?
        if self.flags_hash != flags_hash {
            return FreshResult::Stale("compiler flags changed".into());
        }

        // 2. Artifact exists and mtime matches?
        match file_mtime_secs(&self.artifact_path) {
            Some(mtime) if mtime == self.artifact_mtime_secs => {}
            Some(_) => {
                return FreshResult::Stale("artifact mtime changed".into());
            }
            None => {
                return FreshResult::Stale("artifact missing".into());
            }
        }

        // 3. Any dependency was rebuilt?
        for dep in dep_names {
            if rebuilt_deps.contains(dep) {
                return FreshResult::Stale(format!("dependency `{dep}` was rebuilt"));
            }
        }

        // 4. Check each source file.
        for (path, record) in &mut self.sources {
            let current_mtime = match file_mtime_secs(path) {
                Some(m) => m,
                None => {
                    return FreshResult::Stale(format!(
                        "source file missing: {}",
                        path.display()
                    ));
                }
            };

            // Fast path: mtime unchanged.
            if current_mtime == record.mtime_secs {
                continue;
            }

            // Slow path: hash the file contents.
            let current_hash = match hash_file(path) {
                Ok(h) => h,
                Err(e) => {
                    return FreshResult::Stale(format!(
                        "failed to hash {}: {e}",
                        path.display()
                    ));
                }
            };

            if current_hash != record.content_hash {
                return FreshResult::Stale(format!(
                    "source changed: {}",
                    path.display()
                ));
            }

            // Content unchanged despite mtime change — update stored mtime.
            record.mtime_secs = current_mtime;
        }

        FreshResult::Fresh
    }

    /// Build a cache entry from a just-completed compilation.
    ///
    /// Reads the `.d` dep-info file to discover all source dependencies,
    /// then hashes and records each one.
    pub fn from_compilation(
        flags_hash: String,
        artifact: &Path,
        dep_info: &Path,
    ) -> Result<Self> {
        let artifact_mtime = file_mtime_secs(artifact).unwrap_or(0);

        let source_paths = if dep_info.exists() {
            parse_dep_info(dep_info)?
        } else {
            Vec::new()
        };

        let mut sources = HashMap::new();
        for src in source_paths {
            if !src.exists() {
                continue;
            }
            let mtime = file_mtime_secs(&src).unwrap_or(0);
            let content_hash = hash_file(&src).unwrap_or_default();
            sources.insert(
                src,
                SourceRecord {
                    mtime_secs: mtime,
                    content_hash,
                },
            );
        }

        Ok(Self {
            flags_hash,
            artifact_path: artifact.to_path_buf(),
            artifact_mtime_secs: artifact_mtime,
            sources,
            artifact_hash: None,
        })
    }
}

/// Compute a SHA-256 hash of the `rustc -vV` output to detect toolchain changes.
pub fn get_rustc_version_hash() -> Result<String> {
    Ok(hash_bytes(crate::rustc_info::version_output().as_bytes()))
}

/// Compute a SHA-256 hash of sysroot library entry-point sources.
///
/// Hashes the `lib.rs` files for core, compiler_builtins, and alloc to detect
/// source-level changes that wouldn't be caught by `rustc -vV` alone (e.g.
/// manual patching or toolchain updates with the same version string).
pub fn hash_sysroot_sources(sysroot_src: &Path) -> String {
    let entries = [
        "core/src/lib.rs",
        "compiler_builtins/src/lib.rs",
        "alloc/src/lib.rs",
    ];
    let mut hasher = Sha256::new();
    for entry in &entries {
        let path = sysroot_src.join(entry);
        if let Ok(content) = fs::read(&path) {
            hasher.update(&content);
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Parse a Makefile-style `.d` dep-info file into a list of source paths.
///
/// The format is: `target: dep1 dep2 dep3 ...`
/// with backslash-newline continuations.
pub fn parse_dep_info(path: &Path) -> Result<Vec<PathBuf>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read dep-info: {}", path.display()))?;

    // Join backslash-continuation lines.
    let joined = content.replace("\\\n", " ");

    let mut paths = Vec::new();
    for line in joined.lines() {
        // Skip empty lines.
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Strip the target prefix (everything before the first ':').
        let deps_part = match line.find(':') {
            Some(idx) => &line[idx + 1..],
            None => line,
        };

        // Split on whitespace, handling simple escaped spaces.
        for token in split_dep_tokens(deps_part) {
            let p = PathBuf::from(token);
            if p.extension().is_some() {
                paths.push(p);
            }
        }
    }

    Ok(paths)
}

/// Split a dep-info dependency string on whitespace, handling backslash-escaped spaces.
fn split_dep_tokens(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if next == ' ' {
                    current.push(' ');
                    chars.next();
                    continue;
                }
            }
            current.push(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Get a file's mtime as seconds since the Unix epoch.
pub fn file_mtime_secs(path: &Path) -> Option<i64> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let duration = mtime.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some(duration.as_secs() as i64)
}

/// SHA-256 hash of a file's contents, returned as a hex string.
pub fn hash_file(path: &Path) -> Result<String> {
    let data = fs::read(path)
        .with_context(|| format!("failed to read file for hashing: {}", path.display()))?;
    Ok(hash_bytes(&data))
}

/// SHA-256 hash of a byte slice, returned as a hex string.
pub fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// Create a unique temporary directory for a test, using the test name
    /// and process ID to avoid collisions.
    fn make_test_dir(test_name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("gluon_cache_test_{}_{}", std::process::id(), test_name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("failed to create test temp dir");
        dir
    }

    // ---------------------------------------------------------------
    // split_dep_tokens tests
    // ---------------------------------------------------------------

    #[test]
    fn split_dep_tokens_empty_input() {
        assert_eq!(split_dep_tokens(""), Vec::<String>::new());
    }

    #[test]
    fn split_dep_tokens_simple_tokens() {
        assert_eq!(
            split_dep_tokens("foo.rs bar.rs"),
            vec!["foo.rs", "bar.rs"],
        );
    }

    #[test]
    fn split_dep_tokens_escaped_spaces() {
        assert_eq!(
            split_dep_tokens("path\\ with\\ spaces.rs other.rs"),
            vec!["path with spaces.rs", "other.rs"],
        );
    }

    #[test]
    fn split_dep_tokens_consecutive_whitespace() {
        assert_eq!(
            split_dep_tokens("a.rs    b.rs"),
            vec!["a.rs", "b.rs"],
        );
    }

    #[test]
    fn split_dep_tokens_trailing_whitespace() {
        assert_eq!(
            split_dep_tokens("a.rs "),
            vec!["a.rs"],
        );
    }

    #[test]
    fn split_dep_tokens_backslash_not_before_space() {
        // A backslash followed by 'n' is NOT an escape — keep the backslash.
        assert_eq!(
            split_dep_tokens("path\\nfoo.rs"),
            vec!["path\\nfoo.rs"],
        );
    }

    // ---------------------------------------------------------------
    // parse_dep_info tests
    // ---------------------------------------------------------------

    #[test]
    fn parse_dep_info_standard_colon_target() {
        let dir = make_test_dir("dep_info_standard");
        let dep_file = dir.join("test.d");
        fs::write(&dep_file, "target.rlib: foo.rs bar.rs\n")
            .expect("failed to write dep file");

        let result = parse_dep_info(&dep_file).expect("parse_dep_info failed");
        assert_eq!(result, vec![PathBuf::from("foo.rs"), PathBuf::from("bar.rs")]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_dep_info_backslash_continuations() {
        let dir = make_test_dir("dep_info_continuations");
        let dep_file = dir.join("test.d");
        fs::write(&dep_file, "target.rlib: foo.rs \\\n bar.rs\n")
            .expect("failed to write dep file");

        let result = parse_dep_info(&dep_file).expect("parse_dep_info failed");
        assert_eq!(result, vec![PathBuf::from("foo.rs"), PathBuf::from("bar.rs")]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_dep_info_no_colon_line_still_parsed() {
        let dir = make_test_dir("dep_info_no_colon");
        let dep_file = dir.join("test.d");
        // A line without a colon should still have its tokens parsed as paths.
        fs::write(&dep_file, "src/main.rs src/lib.rs\n")
            .expect("failed to write dep file");

        let result = parse_dep_info(&dep_file).expect("parse_dep_info failed");
        assert_eq!(
            result,
            vec![PathBuf::from("src/main.rs"), PathBuf::from("src/lib.rs")],
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_dep_info_extension_filtering() {
        let dir = make_test_dir("dep_info_ext_filter");
        let dep_file = dir.join("test.d");
        // Paths without extensions (e.g. bare directory names) are excluded.
        fs::write(&dep_file, "target.rlib: foo.rs /usr/lib bar.rs\n")
            .expect("failed to write dep file");

        let result = parse_dep_info(&dep_file).expect("parse_dep_info failed");
        assert_eq!(result, vec![PathBuf::from("foo.rs"), PathBuf::from("bar.rs")]);
        // `/usr/lib` has no extension so it must NOT appear.
        assert!(!result.contains(&PathBuf::from("/usr/lib")));

        let _ = fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // FreshResult tests
    // ---------------------------------------------------------------

    #[test]
    fn fresh_result_fresh_is_fresh() {
        assert!(FreshResult::Fresh.is_fresh());
    }

    #[test]
    fn fresh_result_stale_is_not_fresh() {
        assert!(!FreshResult::Stale("reason".into()).is_fresh());
    }

    // ---------------------------------------------------------------
    // CrateEntry::is_fresh tests
    // ---------------------------------------------------------------

    /// Helper: compute the SHA-256 hex hash of `data`.
    fn sha256_hex(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }

    /// Helper: build a `CrateEntry` with one source file and the given artifact.
    fn make_entry(
        flags_hash: &str,
        artifact: &Path,
        sources: Vec<(PathBuf, i64, String)>,
    ) -> CrateEntry {
        let artifact_mtime = file_mtime_secs(artifact).unwrap_or(0);
        let mut src_map = HashMap::new();
        for (path, mtime, hash) in sources {
            src_map.insert(path, SourceRecord {
                mtime_secs: mtime,
                content_hash: hash,
            });
        }
        CrateEntry {
            flags_hash: flags_hash.to_string(),
            artifact_path: artifact.to_path_buf(),
            artifact_mtime_secs: artifact_mtime,
            sources: src_map,
            artifact_hash: None,
        }
    }

    #[test]
    fn crate_entry_flags_changed_is_stale() {
        let dir = make_test_dir("ce_flags");
        let artifact = dir.join("lib.rlib");
        fs::write(&artifact, b"artifact").unwrap();

        let mut entry = make_entry("old_hash", &artifact, vec![]);
        let result = entry.is_fresh("new_hash", &HashSet::new(), &[]);
        assert!(!result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crate_entry_artifact_missing_is_stale() {
        let dir = make_test_dir("ce_artifact_missing");
        let artifact = dir.join("lib.rlib");
        // Create the artifact so we can record its mtime, then delete it.
        fs::write(&artifact, b"artifact").unwrap();
        let mut entry = make_entry("hash", &artifact, vec![]);
        fs::remove_file(&artifact).unwrap();

        let result = entry.is_fresh("hash", &HashSet::new(), &[]);
        assert!(!result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crate_entry_dependency_rebuilt_is_stale() {
        let dir = make_test_dir("ce_dep_rebuilt");
        let artifact = dir.join("lib.rlib");
        fs::write(&artifact, b"artifact").unwrap();

        let mut entry = make_entry("hash", &artifact, vec![]);
        let mut rebuilt = HashSet::new();
        rebuilt.insert("some_dep".to_string());

        let deps = vec!["some_dep".to_string()];
        let result = entry.is_fresh("hash", &rebuilt, &deps);
        assert!(!result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crate_entry_everything_matches_is_fresh() {
        let dir = make_test_dir("ce_fresh");
        let artifact = dir.join("lib.rlib");
        fs::write(&artifact, b"artifact").unwrap();

        let src = dir.join("main.rs");
        let src_content = b"fn main() {}";
        fs::write(&src, src_content).unwrap();
        let src_mtime = file_mtime_secs(&src).unwrap();
        let src_hash = sha256_hex(src_content);

        let mut entry = make_entry(
            "hash",
            &artifact,
            vec![(src.clone(), src_mtime, src_hash)],
        );

        let result = entry.is_fresh("hash", &HashSet::new(), &[]);
        assert!(result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crate_entry_mtime_changed_but_content_same_is_fresh_and_updates_record() {
        let dir = make_test_dir("ce_mtime_content");
        let artifact = dir.join("lib.rlib");
        fs::write(&artifact, b"artifact").unwrap();

        let src = dir.join("main.rs");
        let src_content = b"fn main() {}";
        fs::write(&src, src_content).unwrap();
        let src_hash = sha256_hex(src_content);

        // Record with a stale mtime (0), but the content hash still matches
        // the file on disk.
        let old_mtime: i64 = 0;
        let mut entry = make_entry(
            "hash",
            &artifact,
            vec![(src.clone(), old_mtime, src_hash)],
        );

        let result = entry.is_fresh("hash", &HashSet::new(), &[]);
        assert!(result.is_fresh(), "should be fresh because content hash matches");

        // The stored mtime should have been updated to the real file mtime.
        let updated_mtime = entry.sources.get(&src).unwrap().mtime_secs;
        let actual_mtime = file_mtime_secs(&src).unwrap();
        assert_eq!(
            updated_mtime, actual_mtime,
            "stored mtime should be updated to current file mtime"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    // ---------------------------------------------------------------
    // CacheManifest::is_sysroot_fresh tests
    // ---------------------------------------------------------------

    #[test]
    fn sysroot_fresh_no_cached_entry_is_stale() {
        let manifest = CacheManifest::new("rustc_hash".into());
        let result = manifest.is_sysroot_fresh("x86_64-unknown-hadron", 2, "hash");
        assert!(!result.is_fresh());
    }

    #[test]
    fn sysroot_fresh_opt_level_changed_is_stale() {
        let dir = make_test_dir("sysroot_opt");
        let core_rlib = dir.join("libcore.rlib");
        let cb_rlib = dir.join("libcompiler_builtins.rlib");
        let alloc_rlib = dir.join("liballoc.rlib");
        fs::write(&core_rlib, b"core").unwrap();
        fs::write(&cb_rlib, b"cb").unwrap();
        fs::write(&alloc_rlib, b"alloc").unwrap();

        let mut manifest = CacheManifest::new("rustc_hash".into());
        manifest.record_sysroot(
            "x86_64-unknown-hadron",
            1,
            core_rlib,
            cb_rlib,
            alloc_rlib,
            "src_hash".into(),
        );

        // Ask for opt-level 2, but recorded 1.
        let result = manifest.is_sysroot_fresh("x86_64-unknown-hadron", 2, "src_hash");
        assert!(!result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sysroot_fresh_sources_changed_is_stale() {
        let dir = make_test_dir("sysroot_src_changed");
        let core_rlib = dir.join("libcore.rlib");
        let cb_rlib = dir.join("libcompiler_builtins.rlib");
        let alloc_rlib = dir.join("liballoc.rlib");
        fs::write(&core_rlib, b"core").unwrap();
        fs::write(&cb_rlib, b"cb").unwrap();
        fs::write(&alloc_rlib, b"alloc").unwrap();

        let mut manifest = CacheManifest::new("rustc_hash".into());
        manifest.record_sysroot(
            "x86_64-unknown-hadron",
            2,
            core_rlib,
            cb_rlib,
            alloc_rlib,
            "old_src_hash".into(),
        );

        let result = manifest.is_sysroot_fresh("x86_64-unknown-hadron", 2, "new_src_hash");
        assert!(!result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sysroot_fresh_empty_stored_hash_is_stale() {
        let dir = make_test_dir("sysroot_empty_hash");
        let core_rlib = dir.join("libcore.rlib");
        let cb_rlib = dir.join("libcompiler_builtins.rlib");
        let alloc_rlib = dir.join("liballoc.rlib");
        fs::write(&core_rlib, b"core").unwrap();
        fs::write(&cb_rlib, b"cb").unwrap();
        fs::write(&alloc_rlib, b"alloc").unwrap();

        let mut manifest = CacheManifest::new("rustc_hash".into());
        // Simulate old manifest with empty sources_hash (migration case).
        manifest.record_sysroot(
            "x86_64-unknown-hadron",
            2,
            core_rlib,
            cb_rlib,
            alloc_rlib,
            String::new(),
        );

        let result = manifest.is_sysroot_fresh("x86_64-unknown-hadron", 2, "any_hash");
        assert!(!result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sysroot_fresh_rlib_missing_is_stale() {
        let dir = make_test_dir("sysroot_missing");
        let core_rlib = dir.join("libcore.rlib");
        let cb_rlib = dir.join("libcompiler_builtins.rlib");
        let alloc_rlib = dir.join("liballoc.rlib");
        fs::write(&core_rlib, b"core").unwrap();
        fs::write(&cb_rlib, b"cb").unwrap();
        fs::write(&alloc_rlib, b"alloc").unwrap();

        let mut manifest = CacheManifest::new("rustc_hash".into());
        manifest.record_sysroot(
            "x86_64-unknown-hadron",
            2,
            core_rlib.clone(),
            cb_rlib,
            alloc_rlib,
            "src_hash".into(),
        );

        // Remove one rlib.
        fs::remove_file(&core_rlib).unwrap();

        let result = manifest.is_sysroot_fresh("x86_64-unknown-hadron", 2, "src_hash");
        assert!(!result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sysroot_fresh_all_present_and_matching_is_fresh() {
        let dir = make_test_dir("sysroot_fresh");
        let core_rlib = dir.join("libcore.rlib");
        let cb_rlib = dir.join("libcompiler_builtins.rlib");
        let alloc_rlib = dir.join("liballoc.rlib");
        fs::write(&core_rlib, b"core").unwrap();
        fs::write(&cb_rlib, b"cb").unwrap();
        fs::write(&alloc_rlib, b"alloc").unwrap();

        let mut manifest = CacheManifest::new("rustc_hash".into());
        manifest.record_sysroot(
            "x86_64-unknown-hadron",
            2,
            core_rlib,
            cb_rlib,
            alloc_rlib,
            "src_hash".into(),
        );

        let result = manifest.is_sysroot_fresh("x86_64-unknown-hadron", 2, "src_hash");
        assert!(result.is_fresh());

        let _ = fs::remove_dir_all(&dir);
    }
}
