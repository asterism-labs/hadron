//! Cached rustc metadata.
//!
//! Lazily queries `rustc -vV` and `rustc --print sysroot` once per process,
//! caching the results for all subsequent callers. Eliminates redundant
//! subprocess invocations that previously occurred across `cache.rs`,
//! `engine.rs`, and `sysroot.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

/// Cached rustc metadata, populated lazily on first access.
struct RustcInfo {
    /// Raw stdout of `rustc -vV`.
    version_output: String,
    /// The `host:` triple extracted from `-vV` output.
    host_triple: String,
    /// The sysroot path from `rustc --print sysroot`.
    sysroot_path: PathBuf,
}

static RUSTC_INFO: OnceLock<RustcInfo> = OnceLock::new();

fn get_info() -> &'static RustcInfo {
    RUSTC_INFO.get_or_init(|| {
        let info = query_rustc().expect("failed to query rustc info");
        info
    })
}

fn query_rustc() -> Result<RustcInfo> {
    // Query version info.
    let vv_output = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("failed to run `rustc -vV`")?;
    let version_output = String::from_utf8(vv_output.stdout)
        .context("non-UTF-8 rustc -vV output")?;

    let host_triple = version_output
        .lines()
        .find(|l| l.starts_with("host:"))
        .and_then(|l| l.strip_prefix("host: "))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".into());

    // Query sysroot path.
    let sysroot_output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .context("failed to run `rustc --print sysroot`")?;

    if !sysroot_output.status.success() {
        bail!(
            "rustc --print sysroot failed: {}",
            String::from_utf8_lossy(&sysroot_output.stderr)
        );
    }

    let sysroot_path = PathBuf::from(
        String::from_utf8(sysroot_output.stdout)
            .context("non-UTF-8 sysroot path")?
            .trim(),
    );

    Ok(RustcInfo {
        version_output,
        host_triple,
        sysroot_path,
    })
}

/// Returns the raw `rustc -vV` output (cached after first call).
pub fn version_output() -> &'static str {
    &get_info().version_output
}

/// Returns the host triple from `rustc -vV` (cached after first call).
pub fn host_triple() -> &'static str {
    &get_info().host_triple
}

/// Returns the sysroot path from `rustc --print sysroot` (cached after first call).
pub fn sysroot_path() -> &'static PathBuf {
    &get_info().sysroot_path
}
