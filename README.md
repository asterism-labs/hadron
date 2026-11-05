# Hadron

Hadron is a monolithic kernel written in Rust. It targets x86_64 first and
aarch64 second.

This branch is a rewrite from scratch. It holds the no_std crates carried
over from the previous kernel and a kernel that boots through Limine, sets
up its own page tables, frame allocator and heap, starts kernel threads on
one CPU, and exits.

## Layout

`crates/` holds libraries that do not depend on Hadron. Another kernel
could use them unchanged. `kernel/` holds the kernel and the crates
written for it.

| Path | Contents |
|------|----------|
| `crates/arch/x86_64` | x86_64 addresses, page tables, descriptor tables and the TSS |
| `crates/boot/uefi` | UEFI type bindings and protocol definitions |
| `crates/boot/limine` | Limine boot protocol bindings |
| `crates/core/linkset` | Access to data collected in linker sections |
| `crates/parse/acpi` | ACPI table parser and static AML namespace walker |
| `crates/parse/dwarf` | DWARF line-program parser |
| `crates/parse/elf` | ELF64 parser and x86_64 relocations |
| `crates/parse/fdt` | Flattened device tree parser |
| `kernel/kernel` | The kernel |
| `kernel/ktest` | Kernel test registration |
| `kernel/mm` | Memory map, frame allocators, page table mapper and heap |

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

## Kernel tests

Kernel tests run inside the kernel after boot. Define one with
`ktest::ktest!` in the kernel crate. A test fails by panicking.

```rust
ktest::ktest! {
    fn null_page_is_unmapped() {
        assert_eq!(translate(VirtAddr::ZERO), None);
    }
}
```

Write `#[ignore]` before `fn` to keep a test out of the default run.

```sh
gluon run -p test                          # run the tests
GLUON_KTEST_IGNORED=true gluon run -p test # also run #[ignore] tests
```

The `test` profile sets the `KTEST` config option. The kernel then prints
one line per test on the serial port and exits QEMU with status 33 when
every test passes. The kernel has no unwinding, so the first failing test
ends the run: the panic handler prints `FAILED` and the panic message, and
QEMU exits with status 35. `gluon run` exits 0 or 1 to match.

## History

- `legacy`: the previous monolithic kernel, with userspace, a Wayland
  compositor and partial Mesa ports.
- `microkernel`: an unfinished capability-based microkernel.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
