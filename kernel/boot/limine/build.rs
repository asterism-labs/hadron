//! Build script for hadron-boot-limine: wires up the linker script.

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .unwrap() // boot/
        .parent()
        .unwrap() // kernel/
        .parent()
        .unwrap(); // workspace root

    let target = std::env::var("TARGET").unwrap_or_default();
    let linker_script = if target.starts_with("x86_64") {
        "x86_64-unknown-hadron.ld"
    } else if target.starts_with("aarch64") {
        "aarch64-unknown-hadron.ld"
    } else {
        return; // native target, no custom linker script
    };

    let script_path = workspace_root.join("targets").join(linker_script);
    println!("cargo:rustc-link-arg-bins=-T{}", script_path.display());
    println!("cargo:rerun-if-changed={}", script_path.display());
}
