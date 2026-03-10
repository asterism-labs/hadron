//! Embedded userboot ELF binary.
//!
//! The userboot binary is compiled for the `x86_64-unknown-hadron-user` target
//! and embedded into the kernel image at build time via `include_bytes!`.
//! Build ordering is guaranteed by `artifact_deps(["userboot"])` in `gluon.rhai`.

/// Raw bytes of the userboot ELF binary.
static USERBOOT_ELF: &[u8] =
    include_bytes!("../../../build/kernel/x86_64-unknown-hadron-user/debug/userboot");

/// Returns the embedded userboot ELF binary.
pub fn elf_bytes() -> &'static [u8] {
    USERBOOT_ELF
}
