//! Code generation command for `cargo xtask codegen`.
//!
//! Reads `codegen.toml` from the workspace root, generates Rust source files
//! for each configured font, and writes them to the specified output paths.

use anyhow::{Context, Result};
use std::path::Path;

use hadron_codegen::config::CodegenConfig;
use hadron_codegen::font;

/// Runs the code generation pipeline.
///
/// Reads `codegen.toml`, generates all configured fonts, and writes output files.
pub fn run_codegen(workspace_root: &Path) -> Result<()> {
    let config_path = workspace_root.join("codegen.toml");
    let config_str = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;
    let config: CodegenConfig =
        toml::from_str(&config_str).context("Failed to parse codegen.toml")?;

    for spec in &config.fonts {
        println!("Generating font '{}'...", spec.name);

        let source = font::generate(spec, workspace_root)
            .with_context(|| format!("Failed to generate font '{}'", spec.name))?;

        let output_path = workspace_root.join(&spec.output);

        // Ensure parent directory exists.
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory {}", parent.display()))?;
        }

        std::fs::write(&output_path, source)
            .with_context(|| format!("Failed to write {}", output_path.display()))?;

        println!("  -> {}", spec.output.display());
    }

    println!("Code generation complete.");
    Ok(())
}
