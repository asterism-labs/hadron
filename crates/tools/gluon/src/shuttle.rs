//! Shuttle randomized concurrency test runner.
//!
//! Runs `RUSTFLAGS="--cfg shuttle" cargo test -p <crate>` in parallel for each
//! crate listed in the `shuttle_testable` configuration, following the same
//! work-stealing pattern as [`crate::kani`].

use anyhow::{Result, bail};
use std::process::Command;

use crate::cli::ShuttleArgs;
use crate::config::ResolvedConfig;

/// Result of a single Shuttle test run.
struct ShuttleResult {
    crate_name: String,
    success: bool,
    output: std::process::Output,
}

/// Run Shuttle randomized concurrency tests on configured crates.
///
/// If `args.package` is set, only that crate is tested (it must be in the
/// `shuttle_testable` list). Otherwise all `shuttle_testable` crates are run in
/// parallel using a work-stealing thread pool.
pub fn run_shuttle(config: &ResolvedConfig, max_workers: usize, args: &ShuttleArgs) -> Result<()> {
    let all_crates = &config.tests.shuttle_testable;
    if all_crates.is_empty() {
        println!("No shuttle-testable crates configured.");
        return Ok(());
    }

    let crates: Vec<String> = if let Some(ref pkg) = args.package {
        if !all_crates.contains(pkg) {
            bail!(
                "crate '{}' is not in shuttle_testable list: [{}]",
                pkg,
                all_crates.join(", ")
            );
        }
        vec![pkg.clone()]
    } else {
        all_crates.clone()
    };

    let num_workers = match max_workers {
        0 => std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4),
        n => n,
    };
    let num_workers = num_workers.min(crates.len());

    println!(
        "Running Shuttle concurrency tests ({} crate{}, {} worker{})...",
        crates.len(),
        if crates.len() == 1 { "" } else { "s" },
        num_workers,
        if num_workers == 1 { "" } else { "s" },
    );

    let root = &config.root;
    let crates = &crates;
    let extra_args = &args.extra_args;
    let next_idx = std::sync::Mutex::new(0usize);
    let (tx, rx) = std::sync::mpsc::channel::<ShuttleResult>();

    std::thread::scope(|s| {
        for _ in 0..num_workers {
            let tx = tx.clone();
            let next = &next_idx;
            s.spawn(move || {
                loop {
                    let idx = {
                        let mut guard = next.lock().unwrap();
                        let i = *guard;
                        if i >= crates.len() {
                            break;
                        }
                        *guard = i + 1;
                        i
                    };

                    let crate_name = &crates[idx];
                    let mut cmd = Command::new("cargo");
                    cmd.arg("test")
                        .arg("-p")
                        .arg(crate_name)
                        .env("RUSTFLAGS", "--cfg shuttle")
                        .current_dir(root);

                    for arg in extra_args {
                        cmd.arg(arg);
                    }

                    let output = cmd.output();

                    let result = match output {
                        Ok(out) => ShuttleResult {
                            crate_name: crate_name.clone(),
                            success: out.status.success(),
                            output: out,
                        },
                        Err(e) => {
                            let stderr = format!("failed to run cargo test with shuttle cfg: {e}");
                            ShuttleResult {
                                crate_name: crate_name.clone(),
                                success: false,
                                output: std::process::Output {
                                    status: std::process::ExitStatus::default(),
                                    stdout: Vec::new(),
                                    stderr: stderr.into_bytes(),
                                },
                            }
                        }
                    };

                    if tx.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        // Collect results as they arrive.
        let mut passed = 0usize;
        let mut failures: Vec<ShuttleResult> = Vec::new();

        for result in rx {
            if result.success {
                println!("  {} ... ok", result.crate_name);
                passed += 1;
            } else {
                println!("  {} ... FAILED", result.crate_name);
                failures.push(result);
            }
        }

        println!(
            "\nShuttle results: {} passed, {} failed",
            passed,
            failures.len()
        );

        if !failures.is_empty() {
            println!("\nFailure details:");
            for f in &failures {
                println!("\n--- {} ---", f.crate_name);
                let stdout = String::from_utf8_lossy(&f.output.stdout);
                let stderr = String::from_utf8_lossy(&f.output.stderr);
                if !stdout.is_empty() {
                    print!("{stdout}");
                }
                if !stderr.is_empty() {
                    eprint!("{stderr}");
                }
            }
            bail!("{} shuttle test(s) failed", failures.len());
        }

        Ok(())
    })
}
