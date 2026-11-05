# Hadron

Hadron is a monolithic kernel written in Rust. It targets x86_64 first and
aarch64 second.

This branch is a rewrite from scratch. It currently holds only the no_std
crates carried over from the previous kernel. The kernel itself has not been
started.

## Layout

| Path | Contents |
|------|----------|
| `crates/boot/uefi` | UEFI type bindings and protocol definitions |
| `crates/boot/limine` | Limine boot protocol bindings |
| `crates/core/linkset` | Access to data collected in linker sections |
| `crates/parse/acpi` | ACPI table parser and static AML namespace walker |
| `crates/parse/binparse` | Binary structure parsing, with derive macros in `binparse-macros` |
| `crates/parse/dwarf` | DWARF line-program parser |
| `crates/parse/elf` | ELF64 parser and x86_64 relocations |
| `crates/parse/fdt` | Flattened device tree parser |

## Development

The dev shell provides rustup, QEMU, prek and taplo. rustup installs the
nightly toolchain from `rust-toolchain.toml`.

```sh
nix develop          # or `direnv allow` with nix-direnv
prek install         # pre-commit and commit-msg hooks
cargo test --workspace
```

The kernel will build with [Gluon](https://github.com/hxyulin/gluon).

The hooks check whitespace, final newlines, `cargo fmt`, `taplo fmt` and
`cargo clippy -D warnings`. Commit subjects must follow
`type(scope): summary`, with at most 72 characters.

## History

- `legacy`: the previous monolithic kernel, with userspace, a Wayland
  compositor and partial Mesa ports.
- `microkernel`: an unfinished capability-based microkernel.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
