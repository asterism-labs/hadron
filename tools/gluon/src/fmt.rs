//! Source formatting via `rustfmt`.
//!
//! Formats all `.rs` files in project directories (kernel/, crates/, userspace/).
//! Excludes vendor/ and build/ directories.

use anyhow::{Context, Result, bail};
use std::process::Command;
use walkdir::WalkDir;

use crate::cli::FmtArgs;
use crate::config;

/// Run `rustfmt` on all project source files.
pub fn cmd_fmt(args: &FmtArgs) -> Result<()> {
    let root = config::find_project_root()?;

    let project_dirs = ["kernel", "crates", "userspace"];
    let mut rs_files: Vec<String> = Vec::new();

    for dir in &project_dirs {
        let dir_path = root.join(dir);
        if !dir_path.exists() {
            continue;
        }

        for entry in WalkDir::new(&dir_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                rs_files.push(
                    path.to_str()
                        .context("non-UTF-8 path")?
                        .to_string(),
                );
            }
        }
    }

    if rs_files.is_empty() {
        println!("No .rs files found to format.");
        return Ok(());
    }

    let mode = if args.check { "Checking" } else { "Formatting" };
    println!("{mode} {} files...", rs_files.len());

    let mut cmd = Command::new("rustfmt");
    cmd.arg("--edition=2024");

    if args.check {
        cmd.arg("--check");
    }

    for f in &rs_files {
        cmd.arg(f);
    }

    let status = cmd
        .status()
        .context("failed to run rustfmt — is it installed?")?;

    if !status.success() {
        if args.check {
            bail!("formatting check failed — run `gluon fmt` to fix");
        } else {
            bail!("rustfmt exited with {status}");
        }
    }

    if args.check {
        println!("All formatting checks passed.");
    } else {
        println!("Formatted {} files.", rs_files.len());
    }

    Ok(())
}
