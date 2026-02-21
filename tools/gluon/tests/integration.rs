//! Integration tests for the gluon build system.
//!
//! These tests invoke the gluon binary as a subprocess against a minimal
//! fixture project. They are marked `#[ignore]` because they require the
//! gluon binary to be pre-built and a working rustc toolchain.
//!
//! Run with: `cargo test --test integration -- --ignored`

use std::path::PathBuf;
use std::process::Command;

/// Locate the compiled gluon binary.
///
/// `cargo test` places the test binary under `target/debug/deps/`. The main
/// binary lives one level up at `target/debug/gluon`.
fn gluon_binary() -> PathBuf {
    let mut path = std::env::current_exe().expect("could not determine test binary path");
    // Go up from deps/ directory to debug/.
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("gluon");
    path
}

/// Path to the minimal fixture project.
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn configure_exits_zero() {
    let fixture = fixture_dir();
    let output = Command::new(gluon_binary())
        .arg("configure")
        .current_dir(&fixture)
        .output()
        .expect("failed to execute gluon configure");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "gluon configure failed (exit={:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code(),
    );

    // configure writes rust-project.json at the project root.
    let rust_project = fixture.join("rust-project.json");
    assert!(
        rust_project.exists(),
        "rust-project.json was not created at {}",
        rust_project.display(),
    );

    // Clean up generated files so the fixture stays pristine.
    let _ = std::fs::remove_file(&rust_project);
    let _ = std::fs::remove_dir_all(fixture.join("build"));
}

#[test]
#[ignore]
fn clean_removes_build_dir() {
    let fixture = fixture_dir();

    // Create a dummy build/ directory to verify clean removes it.
    let build_dir = fixture.join("build");
    std::fs::create_dir_all(&build_dir).expect("failed to create build/ directory");

    let output = Command::new(gluon_binary())
        .arg("clean")
        .current_dir(&fixture)
        .output()
        .expect("failed to execute gluon clean");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "gluon clean failed (exit={:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code(),
    );

    assert!(
        !build_dir.exists(),
        "build/ directory still exists after gluon clean",
    );
}

#[test]
#[ignore]
fn check_succeeds() {
    let fixture = fixture_dir();
    let output = Command::new(gluon_binary())
        .arg("check")
        .current_dir(&fixture)
        .output()
        .expect("failed to execute gluon check");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "gluon check failed (exit={:?}):\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code(),
    );

    // Clean up build artifacts.
    let _ = std::fs::remove_dir_all(fixture.join("build"));
    let _ = std::fs::remove_file(fixture.join("rust-project.json"));
}

#[test]
#[ignore]
fn configure_nonexistent_profile_fails() {
    let fixture = fixture_dir();
    let output = Command::new(gluon_binary())
        .args(["--profile", "nonexistent", "configure"])
        .current_dir(&fixture)
        .output()
        .expect("failed to execute gluon configure with bad profile");

    assert!(
        !output.status.success(),
        "gluon configure with nonexistent profile should have failed but exited with {:?}",
        output.status.code(),
    );

    // Clean up any partial artifacts.
    let _ = std::fs::remove_dir_all(fixture.join("build"));
    let _ = std::fs::remove_file(fixture.join("rust-project.json"));
}
