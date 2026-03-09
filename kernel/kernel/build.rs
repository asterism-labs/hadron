//! Build script for hadron-kernel: wires up linker script for integration tests.

fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();

    // Only emit the linker script for kernel targets (QEMU integration tests).
    // Host targets (e.g., aarch64-apple-darwin) don't need it.
    if !target.contains("hadron") {
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap() // kernel/
        .parent()
        .unwrap(); // workspace root

    let linker_script = if target.starts_with("x86_64") {
        "x86_64-unknown-hadron.ld"
    } else if target.starts_with("aarch64") {
        "aarch64-unknown-hadron.ld"
    } else {
        return;
    };

    let script_path = workspace_root.join("targets").join(linker_script);
    println!("cargo:rustc-link-arg-tests=-T{}", script_path.display());
    println!("cargo:rerun-if-changed={}", script_path.display());
}
