//! Configuration loading from workspace metadata.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// Hadron workspace metadata from Cargo.toml.
#[derive(Debug, Deserialize)]
struct HadronMetadata {
    /// Default target triple for building.
    #[serde(rename = "default-target")]
    default_target: String,
}

/// Workspace configuration.
#[derive(Debug, Deserialize)]
struct WorkspaceConfig {
    workspace: WorkspaceSection,
}

#[derive(Debug, Deserialize)]
struct WorkspaceSection {
    metadata: Option<MetadataSection>,
}

#[derive(Debug, Deserialize)]
struct MetadataSection {
    hadron: Option<HadronMetadata>,
}

/// Build configuration for xtask commands.
#[derive(Debug, Clone)]
pub struct Config {
    /// Workspace root directory.
    pub workspace_root: PathBuf,
    /// Target directory for build artifacts.
    pub target_dir: PathBuf,
    /// Default target triple.
    pub default_target: String,
}

impl Config {
    /// Load configuration from workspace.
    pub fn load() -> Result<Self> {
        let workspace_root = find_workspace_root()?;
        let cargo_toml = workspace_root.join("Cargo.toml");
        let content = std::fs::read_to_string(&cargo_toml)
            .with_context(|| format!("Failed to read {}", cargo_toml.display()))?;

        let config: WorkspaceConfig =
            toml::from_str(&content).context("Failed to parse Cargo.toml")?;

        let default_target = config
            .workspace
            .metadata
            .and_then(|m| m.hadron)
            .map(|h| h.default_target)
            .unwrap_or_else(|| "x86_64-unknown-hadron".to_string());

        let target_dir = workspace_root.join("target");

        Ok(Self {
            workspace_root,
            target_dir,
            default_target,
        })
    }

    /// Get the path to a target specification file.
    pub fn target_spec(&self, target: &str) -> PathBuf {
        self.workspace_root
            .join("targets")
            .join(format!("{target}.json"))
    }
}

/// Find the workspace root by looking for Cargo.toml with [workspace].
fn find_workspace_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir().context("Failed to get current directory")?;

    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            let content = std::fs::read_to_string(&cargo_toml)?;
            if content.contains("[workspace]") {
                return Ok(dir);
            }
        }

        if !dir.pop() {
            anyhow::bail!("Could not find workspace root (no Cargo.toml with [workspace] found)");
        }
    }
}
