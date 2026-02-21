//! Typed builder for rustc command invocations.
//!
//! Wraps `std::process::Command` with ergonomic methods for common
//! rustc flags, reducing duplication across compile.rs and sysroot.rs.

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context, Result, bail};

/// A typed builder for constructing rustc invocations.
pub struct RustcCommandBuilder {
    cmd: Command,
    crate_name_str: String,
}

impl RustcCommandBuilder {
    /// Create a new builder using the given binary (e.g. "rustc" or "clippy-driver").
    pub fn new(binary: &str) -> Self {
        Self {
            cmd: Command::new(binary),
            crate_name_str: String::new(),
        }
    }

    /// Set the crate name (`--crate-name <name>`).
    pub fn crate_name(&mut self, name: &str) -> &mut Self {
        self.crate_name_str = name.to_string();
        self.cmd.arg("--crate-name").arg(name);
        self
    }

    /// Set the Rust edition (`--edition=<ed>`).
    pub fn edition(&mut self, ed: &str) -> &mut Self {
        self.cmd.arg(format!("--edition={ed}"));
        self
    }

    /// Set the crate type (`--crate-type=<ty>`).
    pub fn crate_type(&mut self, ty: &str) -> &mut Self {
        self.cmd.arg(format!("--crate-type={ty}"));
        self
    }

    /// Set the compilation target (`--target <spec>`).
    pub fn target(&mut self, spec: &str) -> &mut Self {
        self.cmd.arg("--target").arg(spec);
        self
    }

    /// Set the sysroot directory (`--sysroot <dir>`).
    pub fn sysroot(&mut self, dir: &Path) -> &mut Self {
        self.cmd.arg("--sysroot").arg(dir);
        self
    }

    /// Set the optimization level (`-Copt-level=<level>`).
    pub fn opt_level(&mut self, level: u32) -> &mut Self {
        self.cmd.arg(format!("-Copt-level={level}"));
        self
    }

    /// Set debug info level (`-Cdebuginfo=<level>`).
    pub fn debug_info(&mut self, level: u32) -> &mut Self {
        self.cmd.arg(format!("-Cdebuginfo={level}"));
        self
    }

    /// Set panic strategy to abort (`-Cpanic=abort`).
    pub fn panic_abort(&mut self) -> &mut Self {
        self.cmd.arg("-Cpanic=abort");
        self
    }

    /// Enable frame pointers (`-Cforce-frame-pointers=yes`).
    pub fn force_frame_pointers(&mut self) -> &mut Self {
        self.cmd.arg("-Cforce-frame-pointers=yes");
        self
    }

    /// Prefer dynamic linking (`-C prefer-dynamic`).
    pub fn prefer_dynamic(&mut self) -> &mut Self {
        self.cmd.arg("-C").arg("prefer-dynamic");
        self
    }

    /// Enable unstable options (`-Zunstable-options`).
    pub fn unstable_options(&mut self) -> &mut Self {
        self.cmd.arg("-Zunstable-options");
        self
    }

    /// Set the output directory (`--out-dir <dir>`).
    pub fn out_dir(&mut self, dir: &Path) -> &mut Self {
        self.cmd.arg("--out-dir").arg(dir);
        self
    }

    /// Set the emit kinds (`--emit=<kinds>`).
    pub fn emit(&mut self, kinds: &str) -> &mut Self {
        self.cmd.arg(format!("--emit={kinds}"));
        self
    }

    /// Add an extern dependency with a path (`--extern <name>=<path>`).
    pub fn add_extern(&mut self, name: &str, path: &Path) -> &mut Self {
        self.cmd
            .arg("--extern")
            .arg(format!("{name}={}", path.display()));
        self
    }

    /// Add an extern dependency without a path (`--extern <name>`).
    pub fn add_extern_no_path(&mut self, name: &str) -> &mut Self {
        self.cmd.arg("--extern").arg(name);
        self
    }

    /// Add a library search path (`-L <dir>`).
    pub fn search_path(&mut self, dir: &Path) -> &mut Self {
        self.cmd.arg("-L").arg(dir);
        self
    }

    /// Add a `--cfg` flag (`--cfg <flag>`).
    pub fn cfg(&mut self, flag: &str) -> &mut Self {
        self.cmd.arg("--cfg").arg(flag);
        self
    }

    /// Add a feature cfg (`--cfg feature="<name>"`).
    pub fn feature(&mut self, name: &str) -> &mut Self {
        self.cmd.arg("--cfg").arg(format!("feature=\"{name}\""));
        self
    }

    /// Add a linker argument (`-Clink-arg=<arg>`).
    pub fn link_arg(&mut self, arg: &str) -> &mut Self {
        self.cmd.arg(format!("-Clink-arg={arg}"));
        self
    }

    /// Set the source file to compile.
    pub fn source(&mut self, file: &Path) -> &mut Self {
        self.cmd.arg(file);
        self
    }

    /// Enable incremental compilation (`-Cincremental=<dir>`).
    pub fn incremental(&mut self, dir: &Path) -> &mut Self {
        self.cmd.arg(format!("-Cincremental={}", dir.display()));
        self
    }

    /// Allow a lint (`--allow <lint>`).
    pub fn allow(&mut self, lint: &str) -> &mut Self {
        self.cmd.arg("--allow").arg(lint);
        self
    }

    /// Warn on a lint (`-W<lint>`).
    pub fn warn(&mut self, lint: &str) -> &mut Self {
        self.cmd.arg(format!("-W{lint}"));
        self
    }

    /// Escape hatch for any arbitrary argument.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.cmd.arg(arg);
        self
    }

    /// Execute the command and return the output.
    pub fn run(&mut self) -> Result<Output> {
        self.cmd
            .output()
            .with_context(|| format!("failed to run rustc for {}", self.crate_name_str))
    }

    /// Execute the command and bail if it fails.
    pub fn run_checked(&mut self, verb: &str) -> Result<Output> {
        let output = self.run()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("failed to {verb} '{}':\n{stderr}", self.crate_name_str);
        }
        Ok(output)
    }
}
