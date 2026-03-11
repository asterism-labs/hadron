//! Embedded userboot ELF and initrd binaries.
//!
//! The userboot binary is compiled for the `x86_64-unknown-hadron-user` target
//! and embedded into the kernel image at build time via `include_bytes!`.
//! Build ordering is guaranteed by `artifact_deps(["userboot"])` in `gluon.rhai`.
//!
//! Additional userspace binaries (test-child, etc.) are also embedded directly
//! for Phase 2b. Phase 3+ will switch to a CPIO initrd archive.

/// Raw bytes of the userboot ELF binary.
static USERBOOT_ELF: &[u8] =
    include_bytes!("../../../build/kernel/x86_64-unknown-hadron-user/debug/userboot");

/// Raw bytes of the test-child ELF binary.
static TEST_CHILD_ELF: &[u8] =
    include_bytes!("../../../build/kernel/x86_64-unknown-hadron-user/debug/test_child");

/// Raw bytes of the test-receiver ELF binary.
static TEST_RECEIVER_ELF: &[u8] =
    include_bytes!("../../../build/kernel/x86_64-unknown-hadron-user/debug/test_receiver");

/// Returns the embedded userboot ELF binary.
pub fn elf_bytes() -> &'static [u8] {
    USERBOOT_ELF
}

/// Look up an embedded binary by path.
///
/// Phase 2: simple match on known binary names.
/// Phase 3+: parse CPIO initrd archive.
pub fn lookup_initrd_binary(path: &str) -> Option<&'static [u8]> {
    // Strip leading path components to get the binary name.
    let name = path.rsplit('/').next().unwrap_or(path);

    match name {
        "test-child" | "test_child" => Some(TEST_CHILD_ELF),
        "test-receiver" | "test_receiver" => Some(TEST_RECEIVER_ELF),
        _ => None,
    }
}
