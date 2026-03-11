//! Script-based test runner for QEMU integration tests.
//!
//! Discovers `.rhai` test scripts, boots a fresh QEMU instance per test,
//! evaluates the script with VM + assertion bindings, and reports results.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::config::ResolvedConfig;
use crate::script;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A discovered script test.
pub struct ScriptTestDef {
    /// Display name (file stem).
    pub name: String,
    /// Absolute path to the `.rhai` file.
    pub path: PathBuf,
    /// Optional per-test configuration extracted from `test_config()`.
    pub config: ScriptTestConfig,
}

/// Per-test configuration, optionally declared via `test_config(#{...})`.
#[derive(Default)]
pub struct ScriptTestConfig {
    /// Extra QEMU arguments for this test.
    pub qemu_extra_args: Vec<String>,
    /// Test timeout in seconds (overrides the global default).
    pub timeout: Option<u64>,
    /// Logical group name for reporting.
    pub group: Option<String>,
    /// Whether to skip this test.
    pub skip: bool,
}

/// The result of running a single script test.
pub struct ScriptTestResult {
    /// Test name.
    pub name: String,
    /// Pass/fail/skip status.
    pub status: TestStatus,
    /// Wall-clock duration.
    pub duration: Duration,
    /// Captured serial log (last N lines on failure).
    pub serial_log: Vec<String>,
    /// Error message on failure.
    pub error: Option<String>,
}

/// Test outcome.
pub enum TestStatus {
    /// Test passed.
    Pass,
    /// Test failed with an error.
    Fail,
    /// Test was skipped.
    Skip,
}

/// Registered sub-test definition (name + function name to call).
type SubTestDefs = Arc<Mutex<Vec<(String, rhai::FnPtr)>>>;

/// Registers `test("name", || { ... })`, `test_group("name", || { ... })`,
/// `setup(|| { ... })`, and `teardown(|| { ... })` on the engine.
///
/// Sub-tests within a single script share one VM boot. During the first
/// eval pass, `test()` captures function pointers. After eval, the runner
/// calls each captured function and records results.
fn register_grouped_test_bindings(engine: &mut rhai::Engine, defs: &SubTestDefs) {
    let d = Arc::clone(defs);
    engine.register_fn("test", move |name: &str, body: rhai::FnPtr| {
        d.lock().unwrap().push((name.to_string(), body));
    });

    // test_group registers the body as a single named group entry.
    let d2 = Arc::clone(defs);
    engine.register_fn("test_group", move |name: &str, body: rhai::FnPtr| {
        d2.lock().unwrap().push((name.to_string(), body));
    });

    // setup/teardown are no-ops during collection; scripts call them
    // inline and they execute immediately.
    engine.register_fn("setup", |body: rhai::FnPtr| {
        // Store for later use — currently no-op as setup runs inline.
        let _ = body;
    });
    engine.register_fn("teardown", |body: rhai::FnPtr| {
        let _ = body;
    });
}

// ---------------------------------------------------------------------------
// Test config frontmatter
// ---------------------------------------------------------------------------

/// Extracts `ScriptTestConfig` from a Rhai script by evaluating it in a
/// sandboxed engine that only exposes `test_config()`.
fn extract_test_config(source: &str) -> ScriptTestConfig {
    let config = Arc::new(Mutex::new(ScriptTestConfig::default()));

    let mut engine = rhai::Engine::new();
    engine.set_max_operations(1000);

    // Register a no-op for every VM binding so the script parses without
    // errors when we evaluate just the config portion.
    let cfg = Arc::clone(&config);
    engine.register_fn("test_config", move |map: rhai::Map| {
        let mut c = cfg.lock().unwrap();
        if let Some(args) = map.get("qemu_args") {
            if let Some(arr) = args.clone().try_cast::<rhai::Array>() {
                c.qemu_extra_args = arr.iter().filter_map(|v| v.clone().try_cast()).collect();
            }
        }
        if let Some(t) = map.get("timeout") {
            if let Some(secs) = t.as_int().ok() {
                c.timeout = Some(secs as u64);
            }
        }
        if let Some(g) = map.get("group") {
            if let Some(s) = g.clone().try_cast::<rhai::ImmutableString>() {
                c.group = Some(s.to_string());
            }
        }
        if let Some(s) = map.get("skip") {
            if let Some(b) = s.as_bool().ok() {
                c.skip = b;
            }
        }
    });

    // Register stub functions so the rest of the script doesn't cause parse
    // errors. We only care about `test_config()`.
    register_stub_functions(&mut engine);

    // Best-effort: if the script fails, we just use defaults.
    let _ = engine.eval::<rhai::Dynamic>(source);

    Arc::try_unwrap(config)
        .unwrap_or_else(|arc| Mutex::new(std::mem::take(&mut *arc.lock().unwrap())))
        .into_inner()
        .unwrap()
}

/// Registers no-op stubs for common VM functions so config extraction
/// doesn't fail on scripts that call `boot()`, `wait_serial()`, etc.
fn register_stub_functions(engine: &mut rhai::Engine) {
    // VM lifecycle
    engine.register_fn("boot", || rhai::Dynamic::UNIT);
    engine.register_fn("quit", || {});
    engine.register_fn("sleep", |_secs: i64| {});

    // Serial I/O
    engine.register_fn("wait_serial", |_p: &str, _t: i64| -> rhai::Dynamic {
        rhai::Dynamic::UNIT
    });
    engine.register_fn("wait_serial_regex", |_p: &str, _t: i64| -> rhai::Dynamic {
        rhai::Dynamic::UNIT
    });
    engine.register_fn("send_serial", |_d: &str| {});
    engine.register_fn("serial_log", || -> rhai::Array { vec![] });
    engine.register_fn("last_serial_lines", |_n: i64| -> rhai::Array { vec![] });

    // QMP
    engine.register_fn("screenshot", |_p: &str| {});
    engine.register_fn("send_key", |_k: &str| {});

    // Exit
    engine.register_fn("wait_exit", |_t: i64| -> i64 { 0 });
    engine.register_fn("assert_exit", |_e: i64| {});

    // Assertions
    engine.register_fn("assert", |_c: bool, _m: &str| {});
    engine.register_fn("assert_eq", |_a: rhai::Dynamic, _b: rhai::Dynamic| {});
    engine.register_fn("assert_ne", |_a: rhai::Dynamic, _b: rhai::Dynamic| {});
    engine.register_fn("assert_contains", |_h: &str, _n: &str| {});
    engine.register_fn("assert_matches", |_s: &str, _p: &str| {});
    engine.register_fn("fail", |_m: &str| {});

    // Grouped tests
    engine.register_fn("test", |_name: &str, _body: rhai::FnPtr| {});
    engine.register_fn("test_group", |_name: &str, _body: rhai::FnPtr| {});
    engine.register_fn("setup", |_body: rhai::FnPtr| {});
    engine.register_fn("teardown", |_body: rhai::FnPtr| {});
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Discovers all `.rhai` test scripts in the given directory.
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn discover_script_tests(dir: &Path) -> Result<Vec<ScriptTestDef>> {
    let mut tests = Vec::new();

    if !dir.is_dir() {
        return Ok(tests);
    }

    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading script tests dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "rhai"))
        .collect();

    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading script test {}", path.display()))?;
        let config = extract_test_config(&source);

        tests.push(ScriptTestDef { name, path, config });
    }

    Ok(tests)
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// Runs a single script test in its own QEMU instance.
///
/// Returns one result for simple scripts, or multiple for scripts using
/// grouped `test("name", || { ... })` calls.
fn run_script_test(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    test: &ScriptTestDef,
    default_timeout: u64,
) -> Vec<ScriptTestResult> {
    let start = Instant::now();

    if test.config.skip {
        return vec![ScriptTestResult {
            name: test.name.clone(),
            status: TestStatus::Skip,
            duration: Duration::ZERO,
            serial_log: vec![],
            error: None,
        }];
    }

    let timeout = test.config.timeout.unwrap_or(default_timeout);

    // Boot a fresh VM.
    let vm = match script::boot_vm(config, kernel_binary, &test.config.qemu_extra_args) {
        Ok(vm) => Arc::new(vm),
        Err(e) => {
            return vec![ScriptTestResult {
                name: test.name.clone(),
                status: TestStatus::Fail,
                duration: start.elapsed(),
                serial_log: vec![],
                error: Some(format!("failed to boot VM: {e}")),
            }];
        }
    };

    let mut engine = rhai::Engine::new();
    engine.set_max_operations(0); // unlimited for real tests
    script::register_vm_bindings(&mut engine, &vm);
    script::register_assertion_bindings(&mut engine);

    // Register grouped test bindings for scripts that use test()/test_group().
    let sub_test_defs: SubTestDefs = Arc::new(Mutex::new(Vec::new()));
    register_grouped_test_bindings(&mut engine, &sub_test_defs);

    // Set a timeout on the engine.
    let deadline = Instant::now() + Duration::from_secs(timeout);
    engine.on_progress(move |_| {
        if Instant::now() >= deadline {
            Some("test timeout".into())
        } else {
            None
        }
    });

    let source = match std::fs::read_to_string(&test.path) {
        Ok(s) => s,
        Err(e) => {
            let _ = vm.quit();
            return vec![ScriptTestResult {
                name: test.name.clone(),
                status: TestStatus::Fail,
                duration: start.elapsed(),
                serial_log: vm.last_serial_lines(20),
                error: Some(format!("failed to read script: {e}")),
            }];
        }
    };

    let ast = match engine.compile(&source) {
        Ok(ast) => ast,
        Err(e) => {
            let _ = vm.quit();
            return vec![ScriptTestResult {
                name: test.name.clone(),
                status: TestStatus::Fail,
                duration: start.elapsed(),
                serial_log: vm.last_serial_lines(20),
                error: Some(format!("compile error: {e}")),
            }];
        }
    };

    let eval_result = engine.eval_ast::<rhai::Dynamic>(&ast);

    // Check if any sub-tests were registered via test()/test_group().
    let defs = std::mem::take(&mut *sub_test_defs.lock().unwrap());

    if defs.is_empty() {
        // No grouped tests — treat the whole script as a single test.
        let serial_log = vm.last_serial_lines(20);
        let _ = vm.quit();

        return match eval_result {
            Ok(_) => vec![ScriptTestResult {
                name: test.name.clone(),
                status: TestStatus::Pass,
                duration: start.elapsed(),
                serial_log: vec![],
                error: None,
            }],
            Err(e) => vec![ScriptTestResult {
                name: test.name.clone(),
                status: TestStatus::Fail,
                duration: start.elapsed(),
                serial_log,
                error: Some(e.to_string()),
            }],
        };
    }

    // Grouped tests: first check if the top-level eval failed.
    if let Err(e) = eval_result {
        let serial_log = vm.last_serial_lines(20);
        let _ = vm.quit();
        return vec![ScriptTestResult {
            name: test.name.clone(),
            status: TestStatus::Fail,
            duration: start.elapsed(),
            serial_log,
            error: Some(format!("script setup error: {e}")),
        }];
    }

    // Execute each captured sub-test function.
    let mut results = Vec::new();
    for (sub_name, fn_ptr) in &defs {
        let sub_start = Instant::now();
        let call_result = fn_ptr.call::<rhai::Dynamic>(&engine, &ast, ());
        let full_name = format!("{}::{sub_name}", test.name);
        match call_result {
            Ok(_) => results.push(ScriptTestResult {
                name: full_name,
                status: TestStatus::Pass,
                duration: sub_start.elapsed(),
                serial_log: vec![],
                error: None,
            }),
            Err(e) => results.push(ScriptTestResult {
                name: full_name,
                status: TestStatus::Fail,
                duration: sub_start.elapsed(),
                serial_log: vm.last_serial_lines(20),
                error: Some(e.to_string()),
            }),
        }
    }

    let _ = vm.quit();
    results
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

/// Prints structured test results.
fn print_results(results: &[ScriptTestResult]) {
    let passed = results
        .iter()
        .filter(|r| matches!(r.status, TestStatus::Pass))
        .count();
    let failed = results
        .iter()
        .filter(|r| matches!(r.status, TestStatus::Fail))
        .count();
    let skipped = results
        .iter()
        .filter(|r| matches!(r.status, TestStatus::Skip))
        .count();
    let total_duration: Duration = results.iter().map(|r| r.duration).sum();

    // Find max name length for alignment.
    let max_name = results.iter().map(|r| r.name.len()).max().unwrap_or(0);

    println!();
    println!("Running {} script tests...", results.len());

    for r in results {
        let dots = ".".repeat(max_name.saturating_sub(r.name.len()) + 3);
        let status = match r.status {
            TestStatus::Pass => "ok",
            TestStatus::Fail => "FAILED",
            TestStatus::Skip => "skipped",
        };
        println!(
            "  {} {} {} ({:.1}s)",
            r.name,
            dots,
            status,
            r.duration.as_secs_f64()
        );
    }

    // Print failure details.
    for r in results {
        if let TestStatus::Fail = r.status {
            println!();
            println!("--- {} ---", r.name);
            if let Some(ref err) = r.error {
                println!("  {err}");
            }
            if !r.serial_log.is_empty() {
                println!("  Last serial:");
                for line in &r.serial_log {
                    println!("    {line}");
                }
            }
        }
    }

    println!();
    println!(
        "Script tests: {passed} passed, {failed} failed, {skipped} skipped ({:.1}s total)",
        total_duration.as_secs_f64()
    );
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Discovers and runs all script tests, optionally filtered by name.
///
/// # Errors
///
/// Returns an error if test discovery fails or if any test fails.
pub fn run_all_script_tests(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    filter: Option<&str>,
) -> Result<()> {
    let tests_dir = config
        .tests
        .script_tests_dir
        .as_ref()
        .map(|d| config.root.join(d));

    let tests_dir = match tests_dir {
        Some(d) => d,
        None => {
            println!("No script_tests_dir configured; skipping script tests.");
            return Ok(());
        }
    };

    let default_timeout = config.tests.script_default_timeout.unwrap_or(30);

    let mut tests = discover_script_tests(&tests_dir)?;

    if let Some(f) = filter {
        tests.retain(|t| t.name.contains(f));
    }

    if tests.is_empty() {
        println!("No script tests found.");
        return Ok(());
    }

    let results: Vec<ScriptTestResult> = tests
        .iter()
        .flat_map(|t| run_script_test(config, kernel_binary, t, default_timeout))
        .collect();

    print_results(&results);

    let any_failed = results.iter().any(|r| matches!(r.status, TestStatus::Fail));
    if any_failed {
        anyhow::bail!("some script tests failed");
    }

    Ok(())
}
