//! Limine bootloader binary fetching and caching.
//!
//! Downloads Limine binary releases from GitHub and caches them locally.
//! Uses `git clone` to fetch a specific tagged release, matching
//! the approach used by `cargo-image-runner`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Handles downloading and caching Limine bootloader binaries.
pub struct LimineCache {
    cache_dir: PathBuf,
    version: String,
}

/// Limine repository URL.
const LIMINE_REPO_URL: &str = "https://github.com/limine-bootloader/limine.git";

impl LimineCache {
    /// Create a new Limine cache.
    ///
    /// `cache_dir` is the parent directory for bootloader caches
    /// (e.g. `target/hadron-runner/cache/bootloaders/`).
    /// `version` is the git tag to fetch (e.g. `"v10.7.0-binary"`).
    #[must_use]
    pub fn new(cache_dir: &Path, version: &str) -> Self {
        Self {
            cache_dir: cache_dir.to_path_buf(),
            version: version.to_string(),
        }
    }

    /// Ensure Limine binaries are available, downloading if necessary.
    ///
    /// Returns the path to the directory containing `limine-bios.sys`,
    /// `limine-bios-cd.bin`, etc.
    ///
    /// # Errors
    ///
    /// Returns an error if the git clone fails or required files are missing.
    pub fn ensure_available(&self) -> Result<PathBuf> {
        let repo_path = self.cache_dir.join(format!("limine-{}", self.version));

        if repo_path.exists() {
            return Ok(repo_path);
        }

        eprintln!(
            "Fetching Limine ({}) from {}...",
            self.version, LIMINE_REPO_URL
        );

        std::fs::create_dir_all(&self.cache_dir).context("creating bootloader cache directory")?;

        // Clone the repository at the specific tag with depth=1 for speed
        let status = std::process::Command::new("git")
            .args([
                "clone",
                "--depth=1",
                "--branch",
                &self.version,
                LIMINE_REPO_URL,
            ])
            .arg(&repo_path)
            .status()
            .context("spawning git clone for Limine")?;

        if !status.success() {
            // Clean up partial clone
            let _ = std::fs::remove_dir_all(&repo_path);
            bail!(
                "git clone failed for Limine {} (exit code {:?})",
                self.version,
                status.code()
            );
        }

        // Verify required BIOS files exist
        for required in ["limine-bios.sys", "limine-bios-cd.bin"] {
            if !repo_path.join(required).exists() {
                let _ = std::fs::remove_dir_all(&repo_path);
                bail!(
                    "{required} not found in Limine {}. \
                     Make sure you're using a binary release (e.g., v10.7.0-binary).",
                    self.version
                );
            }
        }

        eprintln!("Fetched Limine ({}) successfully", self.version);
        Ok(repo_path)
    }
}
