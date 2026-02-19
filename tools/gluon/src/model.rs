//! Build model types produced by Rhai script evaluation.
//!
//! These are pure data types with no Rhai dependencies. The Rhai engine
//! populates a [`BuildModel`] which is then validated, resolved, and
//! handed to the compilation and scheduling pipeline.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// The complete build model produced by evaluating `gluon.rhai`.
///
/// Contains all declarations: targets, crates, groups, rules, pipeline,
/// configuration options, profiles, and auxiliary settings.
#[derive(Debug, Default)]
pub struct BuildModel {
    pub project: ProjectDef,
    pub targets: BTreeMap<String, TargetDef>,
    pub config_options: BTreeMap<String, ConfigOptionDef>,
    /// Menu category ordering for TUI menuconfig (first-appearance order).
    pub menu_order: Vec<String>,
    pub profiles: BTreeMap<String, ProfileDef>,
    pub crates: BTreeMap<String, CrateDef>,
    pub groups: BTreeMap<String, GroupDef>,
    pub rules: BTreeMap<String, RuleDef>,
    pub pipeline: PipelineDef,
    pub qemu: QemuDef,
    pub bootloader: BootloaderDef,
    pub image: ImageDef,
    pub tests: TestsDef,
    /// External dependency declarations from `dependency()` calls in gluon.rhai.
    pub dependencies: BTreeMap<String, ExternalDepDef>,
}

/// Project metadata.
#[derive(Debug, Default, Clone)]
pub struct ProjectDef {
    pub name: String,
    pub version: String,
}

/// A compilation target definition.
#[derive(Debug, Clone)]
pub struct TargetDef {
    #[allow(dead_code)] // used by validation
    pub name: String,
    pub spec: String,
}

/// Crate output type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateType {
    Lib,
    Bin,
    ProcMacro,
}

impl Default for CrateType {
    fn default() -> Self {
        Self::Lib
    }
}

impl CrateType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lib => "lib",
            Self::Bin => "bin",
            Self::ProcMacro => "proc-macro",
        }
    }
}

/// A typed configuration option (Kconfig-style).
#[derive(Debug, Clone)]
pub struct ConfigOptionDef {
    #[allow(dead_code)] // used by validation
    pub name: String,
    pub ty: ConfigType,
    pub default: ConfigValue,
    pub help: Option<String>,
    pub depends_on: Vec<String>,
    pub selects: Vec<String>,
    pub range: Option<(u64, u64)>,
    pub choices: Option<Vec<String>>,
    /// Menu category for TUI menuconfig grouping.
    pub menu: Option<String>,
}

/// Configuration option type tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigType {
    Bool,
    U32,
    U64,
    Str,
    /// Dedicated enum type with a fixed set of named variants.
    Choice,
    /// Ordered list of strings.
    List,
    /// Nested config group using flat dot-notation keys (e.g. `uart.baud`).
    Group,
}

/// A typed configuration value.
#[derive(Debug, Clone)]
pub enum ConfigValue {
    Bool(bool),
    U32(u32),
    U64(u64),
    Str(String),
    /// Selected variant name for a `ConfigType::Choice` option.
    Choice(String),
    /// Ordered list of string items for a `ConfigType::List` option.
    List(Vec<String>),
}

impl Default for ConfigValue {
    fn default() -> Self {
        Self::Bool(false)
    }
}

/// A build profile definition.
#[derive(Debug, Clone, Default)]
pub struct ProfileDef {
    #[allow(dead_code)] // used by validation
    pub name: String,
    pub inherits: Option<String>,
    pub target: Option<String>,
    pub opt_level: Option<u32>,
    pub debug_info: Option<bool>,
    pub lto: Option<String>,
    pub boot_binary: Option<String>,
    pub config: BTreeMap<String, ConfigValue>,
    pub qemu_memory: Option<u32>,
    pub qemu_cores: Option<u32>,
    pub qemu_extra_args: Option<Vec<String>>,
    pub test_timeout: Option<u32>,
}

/// A crate definition.
#[derive(Debug, Clone)]
pub struct CrateDef {
    #[allow(dead_code)] // used by validation
    pub name: String,
    pub path: String,
    pub edition: String,
    pub crate_type: CrateType,
    /// Target for this crate (inherited from group). `"host"` = host triple.
    pub target: String,
    pub deps: BTreeMap<String, DepDef>,
    pub dev_deps: BTreeMap<String, DepDef>,
    pub features: Vec<String>,
    pub root: Option<String>,
    /// Per-crate linker script (e.g. for kernel binary crates).
    pub linker_script: Option<String>,
    /// The group this crate belongs to.
    #[allow(dead_code)] // used by future group-based queries
    pub group: Option<String>,
    /// Whether this crate is a project crate (for clippy linting).
    pub is_project_crate: bool,
    /// Extra `--cfg` flags for this crate (e.g. `wrap_proc_macro` for proc-macro2).
    pub cfg_flags: Vec<String>,
}

/// A dependency specification within a crate definition.
#[derive(Debug, Clone)]
pub struct DepDef {
    #[allow(dead_code)] // used by crate_graph resolution
    pub extern_name: String,
    pub crate_name: String,
    #[allow(dead_code)] // used by future feature-gated compilation
    pub features: Vec<String>,
}

/// Source location for an external dependency.
#[derive(Debug, Clone)]
pub enum DepSource {
    /// crates.io with exact version.
    CratesIo { version: String },
    /// Git repository.
    Git { url: String, reference: GitRef },
    /// Local path (not vendored, used in-place).
    Path { path: String },
}

/// Git reference type for git-sourced dependencies.
#[derive(Debug, Clone)]
pub enum GitRef {
    /// Exact commit hash.
    Rev(String),
    /// Git tag.
    Tag(String),
    /// Branch name.
    Branch(String),
    /// HEAD of the default branch.
    Default,
}

/// An external dependency declaration from `gluon.rhai`.
#[derive(Debug, Clone)]
pub struct ExternalDepDef {
    pub name: String,
    pub source: DepSource,
    pub features: Vec<String>,
    pub default_features: bool,
    /// Extra `--cfg` flags to pass when compiling this dependency.
    pub cfg_flags: Vec<String>,
}

/// A group of crates with shared compilation behavior.
#[derive(Debug, Clone)]
pub struct GroupDef {
    #[allow(dead_code)] // used by validation
    pub name: String,
    /// Target for all crates in this group. `"host"` = host triple.
    pub target: String,
    pub default_edition: String,
    pub crates: Vec<String>,
    #[allow(dead_code)] // used by future shared-flag compilation
    pub shared_flags: Vec<String>,
    /// Whether crates in this group are project crates (for clippy linting).
    pub is_project: bool,
    /// Whether crates in this group should be linked with the config crate.
    pub config: bool,
}

impl Default for GroupDef {
    fn default() -> Self {
        Self {
            name: String::new(),
            target: "host".into(),
            default_edition: "2024".into(),
            crates: Vec::new(),
            shared_flags: Vec::new(),
            is_project: true,
            config: false,
        }
    }
}

/// A rule for custom artifact generation.
#[derive(Debug, Clone)]
pub struct RuleDef {
    #[allow(dead_code)] // used by validation
    pub name: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub depends_on: Vec<String>,
    pub handler: RuleHandler,
}

/// How a rule's artifact is generated.
#[derive(Debug, Clone)]
pub enum RuleHandler {
    /// A built-in Rust function identified by name (e.g. "hbtf", "initrd", "config_crate").
    Builtin(String),
    /// A Rhai function name to call for user-defined rules.
    #[allow(dead_code)] // used by future script rule handler
    Script(String),
}

/// The build pipeline definition.
#[derive(Debug, Default, Clone)]
pub struct PipelineDef {
    pub steps: Vec<PipelineStep>,
}

/// A single step in the build pipeline.
#[derive(Debug, Clone)]
pub enum PipelineStep {
    /// Compile groups of crates (DAG-scheduled within the stage).
    Stage { name: String, groups: Vec<String> },
    /// Synchronization barrier: wait for all preceding work.
    #[allow(dead_code)] // barrier name used for logging
    Barrier(String),
    /// Execute a named rule.
    Rule(String),
}

/// QEMU configuration.
#[derive(Debug, Clone)]
pub struct QemuDef {
    pub machine: String,
    pub memory: u32,
    pub extra_args: Vec<String>,
    pub test: QemuTestDef,
}

impl Default for QemuDef {
    fn default() -> Self {
        Self {
            machine: "q35".into(),
            memory: 256,
            extra_args: vec!["-serial".into(), "stdio".into()],
            test: QemuTestDef::default(),
        }
    }
}

/// QEMU test configuration.
#[derive(Debug, Clone)]
pub struct QemuTestDef {
    pub success_exit_code: u32,
    pub timeout: u32,
    pub extra_args: Vec<String>,
}

impl Default for QemuTestDef {
    fn default() -> Self {
        Self {
            success_exit_code: 33,
            timeout: 30,
            extra_args: Vec::new(),
        }
    }
}

/// Bootloader configuration.
#[derive(Debug, Clone)]
pub struct BootloaderDef {
    pub kind: String,
    pub config_file: Option<String>,
}

impl Default for BootloaderDef {
    fn default() -> Self {
        Self {
            kind: "limine".into(),
            config_file: Some("limine.conf".into()),
        }
    }
}

/// Image configuration.
#[derive(Debug, Clone, Default)]
pub struct ImageDef {
    pub extra_files: BTreeMap<String, String>,
}

/// Test configuration.
#[derive(Debug, Clone, Default)]
pub struct TestsDef {
    pub host_testable: Vec<String>,
    pub kernel_tests_dir: Option<String>,
    /// Which crate owns the kernel integration tests.
    pub kernel_tests_crate: Option<String>,
    /// Linker script for kernel test binaries.
    pub kernel_tests_linker_script: Option<String>,
    pub crash_tests: Vec<CrashTestDef>,
}

/// A crash test definition.
#[derive(Debug, Clone)]
pub struct CrashTestDef {
    pub name: String,
    pub source: String,
    pub expected_exit: u32,
    pub expect_output: Option<String>,
}

// --- Conversion helpers ---

impl CrateDef {
    /// Determine the root source file for this crate.
    pub fn root_file(&self, resolved_path: &std::path::Path) -> PathBuf {
        if let Some(ref root) = self.root {
            resolved_path.join(root)
        } else {
            match self.crate_type {
                CrateType::Bin => resolved_path.join("src/main.rs"),
                _ => resolved_path.join("src/lib.rs"),
            }
        }
    }
}
