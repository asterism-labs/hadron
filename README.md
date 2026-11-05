# Hadron

Hadron is a monolithic kernel written in Rust. It targets x86_64 first and
aarch64 second.

This branch is a rewrite from scratch. It holds the no_std crates carried
over from the previous kernel and a placeholder kernel that boots through
Limine, prints one line on the serial port and exits.

## Layout

| Path | Contents |
|------|----------|
| `crates/boot/uefi` | UEFI type bindings and protocol definitions |
| `crates/boot/limine` | Limine boot protocol bindings |
| `crates/core/linkset` | Access to data collected in linker sections |
| `crates/kernel/kernel` | Placeholder kernel |
| `crates/kernel/ktest` | Kernel test registration |
| `crates/parse/acpi` | ACPI table parser and static AML namespace walker |
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

The hooks check whitespace, final newlines, `cargo fmt`, `taplo fmt` and
`cargo clippy -D warnings`. Commit messages must follow
[Conventional Commits](https://www.conventionalcommits.org/).

## Building and running

[Gluon](https://github.com/hxyulin/gluon) builds the crates for
`x86_64-unknown-none` and boots the kernel under QEMU with Limine and OVMF.
`gluon.rs` holds the build, and `limine.conf` the boot menu.

```sh
gluon build          # build every crate into build/
gluon clippy         # lint every crate for the kernel target
gluon run            # boot the kernel under QEMU
```

`gluon run` exits 0 when the kernel reports success through QEMU's
`isa-debug-exit` device. The first build downloads Limine's `BOOTX64.EFI`
and checks it against the SHA-256 in `gluon.rs`.

## History

- `legacy`: the previous monolithic kernel, with userspace, a Wayland
  compositor and partial Mesa ports.
- `microkernel`: an unfinished capability-based microkernel.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
